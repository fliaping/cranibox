import { invoke } from '@tauri-apps/api/core';

type Snapshot = { ready: boolean; error: string | null };

const status = document.getElementById('status')!;
const failure = document.getElementById('failure')!;
const message = document.getElementById('error')!;
let timer: number | undefined;

async function poll() {
  window.clearTimeout(timer);
  try {
    const snapshot = await invoke<Snapshot>('snapshot');
    if (snapshot.error) {
      status.hidden = true;
      failure.hidden = false;
      message.textContent = snapshot.error;
      return;
    }
    // The app navigates this window to the interface once it is ready.
    if (snapshot.ready) status.textContent = '正在打开… · Opening…';
  } catch (error) {
    status.textContent = String(error);
  }
  timer = window.setTimeout(poll, 1000);
}

document.getElementById('retry')!.addEventListener('click', () => {
  failure.hidden = true;
  status.hidden = false;
  void poll();
});
document.getElementById('quit')!.addEventListener('click', () => void invoke('quit'));
void poll();
