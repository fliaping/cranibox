//! Keep reading an unpaced primary display while a viewer is slow. Only JPEG
//! damage fully covered by newer damage may be discarded; other messages and
//! resolution barriers retain their order. Clipboard controls are independent.
use super::{MAX_CHUNK, MAX_FRAME};
use anyhow::{Result, ensure};
use std::collections::VecDeque;

#[derive(Clone, Copy)]
struct Stripe {
    width: u16,
    start: u16,
    end: u16,
}
struct Frame {
    data: Vec<u8>,
    stripe: Option<Stripe>,
}
#[derive(Default)]
pub(super) struct Output {
    frames: VecDeque<Frame>,
    clipboard: VecDeque<Frame>,
    active: Option<Frame>,
    offset: usize,
    bytes: usize,
}
impl Output {
    pub(super) fn is_empty(&self) -> bool {
        self.bytes == 0
    }
    pub(super) fn push(&mut self, kind: u8, payload: Vec<u8>) -> Result<()> {
        ensure!(payload.len() <= MAX_FRAME, "Selkies frame too large");
        // This viewer does not implement audio or upstream cursor bitmaps.
        // Selkies can broadcast cursor updates between every group of stripes;
        // carrying ignored updates would prevent damage from being coalesced.
        if (kind == 1 && payload.first() == Some(&1))
            || (kind == 0 && payload.starts_with(b"cursor,"))
        {
            return Ok(());
        }
        let stripe = (kind == 1).then(|| stripe(&payload)).flatten();
        if let Some(new) = stripe {
            // Never cross a resolution/control barrier or discard a stripe
            // already partially sent. Partial overlap still needs both paints.
            let barrier = self.frames.iter().rposition(|f| f.stripe.is_none());
            let mut index = 0;
            self.frames.retain(|frame| {
                let after_barrier = barrier.is_none_or(|barrier| index > barrier);
                index += 1;
                let covered = after_barrier
                    && frame.stripe.is_some_and(|old| {
                        old.width == new.width && new.start <= old.start && new.end >= old.end
                    });
                if covered {
                    self.bytes -= frame.data.len();
                }
                !covered
            });
        }
        let clipboard = kind == 0 && payload.starts_with(b"clipboard");
        ensure!(
            self.frames.len() + self.clipboard.len() < 256
                && self.bytes + 5 + payload.len() <= 2 * MAX_FRAME,
            "Selkies output queue exceeded its bounded capacity"
        );
        let mut data = Vec::with_capacity(5 + payload.len());
        data.push(kind);
        data.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        data.extend_from_slice(&payload);
        self.bytes += data.len();
        let frame = Frame { data, stripe };
        if clipboard {
            self.clipboard.push_back(frame);
        } else {
            self.frames.push_back(frame);
        }
        Ok(())
    }
    pub(super) fn next_chunk(&mut self) -> Vec<u8> {
        if self.active.is_none() {
            self.active = self
                .clipboard
                .pop_front()
                .or_else(|| self.frames.pop_front());
            self.offset = 0;
        }
        let frame = self.active.as_ref().expect("nonempty output");
        let end = (self.offset + MAX_CHUNK).min(frame.data.len());
        let chunk = frame.data[self.offset..end].to_vec();
        self.bytes -= chunk.len();
        self.offset = end;
        if end == frame.data.len() {
            self.active = None;
        }
        chunk
    }
}
fn stripe(data: &[u8]) -> Option<Stripe> {
    if data.first() != Some(&3) {
        return None;
    }
    let start = u16::from_be_bytes(data.get(4..6)?.try_into().ok()?);
    let (width, height) = jpeg_size(data.get(6..)?)?;
    let end = start.checked_add(height)?;
    (width > 0 && width <= 8192 && height > 0 && end <= 8192).then_some(Stripe {
        width,
        start,
        end,
    })
}
// Read only the bounded JPEG header, without decoding pixels or allocating.
fn jpeg_size(data: &[u8]) -> Option<(u16, u16)> {
    if data.get(..2)? != [0xff, 0xd8] {
        return None;
    }
    let mut offset = 2;
    loop {
        if *data.get(offset)? != 0xff {
            return None;
        }
        while data.get(offset) == Some(&0xff) {
            offset += 1;
        }
        let marker = *data.get(offset)?;
        offset += 1;
        if matches!(marker, 0 | 0x01 | 0xd0..=0xda) {
            return None;
        }
        let length = u16::from_be_bytes(data.get(offset..offset + 2)?.try_into().ok()?) as usize;
        if length < 2 {
            return None;
        }
        let segment = data.get(offset..offset + length)?;
        if matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf) {
            if segment.len() < 8 {
                return None;
            }
            return Some((
                u16::from_be_bytes(segment[5..7].try_into().ok()?),
                u16::from_be_bytes(segment[3..5].try_into().ok()?),
            ));
        }
        offset += length;
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    pub(crate) fn jpeg(id: u16, y: u16, width: u16, height: u16) -> Vec<u8> {
        let mut data = vec![3, 0];
        data.extend_from_slice(&id.to_be_bytes());
        data.extend_from_slice(&y.to_be_bytes());
        data.extend_from_slice(&[0xff, 0xd8, 0xff, 0xc0, 0, 8, 8]);
        data.extend_from_slice(&height.to_be_bytes());
        data.extend_from_slice(&width.to_be_bytes());
        data.push(0);
        data
    }
    fn drain(output: &mut Output) -> Vec<Vec<u8>> {
        let mut frames = Vec::new();
        while !output.is_empty() {
            frames.push(output.next_chunk());
        }
        frames
    }
    #[test]
    fn coalesces_only_fully_covered_damage_without_crossing_barriers() {
        let mut output = Output::default();
        output.push(1, jpeg(1, 0, 160, 100)).unwrap();
        output.push(1, jpeg(2, 0, 160, 50)).unwrap();
        assert_eq!(output.frames.len(), 2); // Partial coverage needs the old bottom half.
        output.push(1, jpeg(3, 0, 160, 100)).unwrap();
        assert_eq!(output.frames.len(), 1);
        output.push(0, b"resolution barrier".to_vec()).unwrap();
        output.push(1, jpeg(4, 0, 160, 100)).unwrap();
        assert_eq!(output.frames.len(), 3);
        output.push(1, jpeg(5, 0, 320, 100)).unwrap();
        assert_eq!(output.frames.len(), 4); // Different widths cannot replace each other.
        let frames = drain(&mut output);
        assert_eq!(u16::from_be_bytes(frames[0][7..9].try_into().unwrap()), 3);
        assert_eq!(output.bytes, 0);
    }
    #[test]
    fn prioritizes_clipboard_in_order_without_interleaving_partial_frames() {
        let mut output = Output::default();
        let mut large = jpeg(1, 0, 160, 100);
        large.resize(MAX_CHUNK + 100, 0);
        output.push(1, large).unwrap();
        assert_eq!(output.next_chunk().len(), MAX_CHUNK);
        output.push(1, jpeg(2, 0, 160, 100)).unwrap();
        for text in [
            "clipboard_start,text/plain,1",
            "clipboard_data,YQ==",
            "clipboard_finish",
        ] {
            output.push(0, text.as_bytes().to_vec()).unwrap();
        }
        let frames = drain(&mut output);
        assert_eq!(frames[0].len(), 105);
        assert_eq!(&frames[1][5..], b"clipboard_start,text/plain,1");
        assert_eq!(&frames[3][5..], b"clipboard_finish");
        assert_eq!(frames[4][0], 1);
    }
    #[test]
    fn rejects_unbounded_controls_and_ignores_invalid_jpeg_headers() {
        let mut output = Output::default();
        for _ in 0..256 {
            output.push(0, b"clipboard,".to_vec()).unwrap();
        }
        assert!(output.push(0, b"clipboard,".to_vec()).is_err());
        let mut output = Output::default();
        output.push(1, vec![0; MAX_FRAME]).unwrap();
        assert!(output.push(1, vec![0; MAX_FRAME]).is_err());
        let valid = jpeg(1, 20, 160, 100);
        assert_eq!(jpeg_size(&valid[6..]), Some((160, 100)));
        for end in 0..valid.len() {
            assert!(stripe(&valid[..end]).is_none());
        }
        let mut invalid = valid.clone();
        invalid[10..12].copy_from_slice(&[0, 0]);
        assert!(stripe(&invalid).is_none());
        assert!(stripe(&jpeg(1, 8190, 160, 100)).is_none());
    }
    #[test]
    fn ignored_cursor_and_audio_broadcasts_do_not_block_coalescing() {
        let mut output = Output::default();
        for id in 0..1000 {
            output.push(0, b"cursor,ignored bitmap".to_vec()).unwrap();
            output.push(1, vec![1, 0, 0]).unwrap();
            output.push(1, jpeg(id, 0, 1024, 192)).unwrap();
            output.push(1, jpeg(id, 192, 1024, 192)).unwrap();
        }
        assert_eq!(output.frames.len(), 2);
        let frames = drain(&mut output);
        assert_eq!(u16::from_be_bytes(frames[0][7..9].try_into().unwrap()), 999);
    }
}
