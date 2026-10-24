use std::{cmp::Ordering, fmt};

use anyhow::{Result, ensure};

/// A strict SemVer 2.0 version, limited to 64 ASCII bytes.
/// Equality and ordering ignore build metadata, just like precedence.
#[derive(Clone, Debug)]
pub struct Version {
    core: [String; 3],
    prerelease: Vec<String>,
    build: Option<String>,
}

impl Version {
    pub fn parse(value: &str) -> Result<Self> {
        ensure!(
            !value.is_empty() && value.len() <= 64,
            "version must be 1..=64 bytes"
        );
        ensure!(
            value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".-+".contains(&b)),
            "invalid version character"
        );
        let (value, build) = match value.split_once('+') {
            Some((value, build)) => {
                validate_identifiers(build, false)?;
                (value, Some(build.to_owned()))
            }
            None => (value, None),
        };
        let (core, prerelease) = match value.split_once('-') {
            Some((core, prerelease)) => {
                validate_identifiers(prerelease, true)?;
                (core, prerelease.split('.').map(str::to_owned).collect())
            }
            None => (value, Vec::new()),
        };
        let core: Vec<_> = core.split('.').map(str::to_owned).collect();
        ensure!(core.len() == 3, "version needs major.minor.patch");
        for number in &core {
            ensure!(is_numeric(number), "invalid version number");
            ensure!(
                number.len() == 1 || !number.starts_with('0'),
                "leading zero in version number"
            );
        }
        Ok(Self {
            core: core.try_into().expect("three version components"),
            prerelease,
            build,
        })
    }

    pub fn is_dev(&self) -> bool {
        self.prerelease
            .first()
            .is_some_and(|id| id.starts_with("dev"))
    }
}

fn is_numeric(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit())
}

fn validate_identifiers(value: &str, prerelease: bool) -> Result<()> {
    for id in value.split('.') {
        ensure!(
            !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'),
            "invalid version identifier"
        );
        ensure!(
            !prerelease || !is_numeric(id) || id.len() == 1 || !id.starts_with('0'),
            "leading zero in prerelease identifier"
        );
    }
    Ok(())
}

fn compare_number(left: &str, right: &str) -> Ordering {
    left.len().cmp(&right.len()).then_with(|| left.cmp(right))
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.core[0], self.core[1], self.core[2])?;
        if !self.prerelease.is_empty() {
            write!(f, "-{}", self.prerelease.join("."))?;
        }
        if let Some(build) = &self.build {
            write!(f, "+{build}")?;
        }
        Ok(())
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        for (left, right) in self.core.iter().zip(&other.core) {
            let order = compare_number(left, right);
            if order != Ordering::Equal {
                return order;
            }
        }
        match (self.prerelease.is_empty(), other.prerelease.is_empty()) {
            (true, false) => return Ordering::Greater,
            (false, true) => return Ordering::Less,
            _ => {}
        }
        for (left, right) in self.prerelease.iter().zip(&other.prerelease) {
            let order = match (is_numeric(left), is_numeric(right)) {
                (true, true) => compare_number(left, right),
                (true, false) => Ordering::Less,
                (false, true) => Ordering::Greater,
                (false, false) => left.cmp(right),
            };
            if order != Ordering::Equal {
                return order;
            }
        }
        self.prerelease.len().cmp(&other.prerelease.len())
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Version {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Version {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        for value in [
            "0.0.0",
            "1.2.0-dev.9",
            "1.2.0-dev.17",
            "1.2.0",
            "1.0.0-alpha-1.9+build.001",
            "999999999999999999999999999999.0.0",
        ] {
            let version = Version::parse(value).unwrap();
            assert_eq!(version.to_string(), value);
            assert_eq!(Version::parse(&version.to_string()).unwrap(), version);
        }
        assert!(Version::parse("1.0.0-dev.1").unwrap().is_dev());
        assert!(Version::parse("1.0.0-development").unwrap().is_dev());
        assert!(!Version::parse("1.0.0+dev").unwrap().is_dev());
        assert!(!Version::parse("1.0.0-alpha.dev").unwrap().is_dev());
        let boundary = format!("1.0.0+{}", "x".repeat(58));
        assert_eq!(boundary.len(), 64);
        assert!(Version::parse(&boundary).is_ok());
        assert!(Version::parse(&(boundary + "x")).is_err());
    }

    #[test]
    fn precedence() {
        for values in [
            vec!["1.2.0-dev.9", "1.2.0-dev.17", "1.2.0"],
            vec![
                "1.0.0-alpha",
                "1.0.0-alpha.1",
                "1.0.0-alpha.beta",
                "1.0.0-beta",
                "1.0.0-beta.2",
                "1.0.0-beta.11",
                "1.0.0-rc.1",
                "1.0.0",
            ],
            vec![
                "1.0.0-9", "1.0.0-17", "1.0.0-a", "1.0.0", "1.0.1", "1.1.0", "2.0.0", "10.0.0",
            ],
            vec!["999999999999999999999.0.0", "1000000000000000000000.0.0"],
        ] {
            let versions: Vec<_> = values.iter().map(|s| Version::parse(s).unwrap()).collect();
            for pair in versions.windows(2) {
                assert!(pair[0] < pair[1], "{} < {}", pair[0], pair[1]);
            }
        }
        let a = Version::parse("1.0.0+a").unwrap();
        let b = Version::parse("1.0.0+b").unwrap();
        assert_eq!(a, b);
        assert_eq!(a.cmp(&b), Ordering::Equal);
    }

    #[test]
    fn rejects_bad_input() {
        for value in [
            "",
            "1",
            "1.2",
            "1.2.3.4",
            "v1.2.3",
            "01.2.3",
            "1.02.3",
            "1.2.03",
            "1.2.3-01",
            "1.2.3-a.00",
            "1.2.3-",
            "1.2.3+",
            "1.2.3-a..b",
            "1.2.3+a..b",
            "1.2.3+a+b",
            "../1.2.3",
            "1.2.3/a",
            r"1.2.3\a",
            "1.2.3 a",
            " 1.2.3",
            "1.2.3\n",
            "1.2.3-é",
            "1.2.3_a",
        ] {
            assert!(Version::parse(value).is_err(), "accepted {value:?}");
        }
    }
}
