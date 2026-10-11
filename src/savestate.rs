//! Save states for the emulator savers, one per ROM: files under
//! `SAVER_STATE_DIR/<saver>/` when it is set, else memory for the life of
//! the process.
//!
//! Each ROM keeps its newest state and the one before it, so a state that
//! turns out bad (unreadable, or it crashes the game) falls back a step
//! rather than all the way to power-on. A file is written beside its final
//! name and renamed over it, so a crash mid-write leaves the old one whole;
//! the checksum catches what that cannot (a torn disk, a hand edit), and the
//! format tag a core upgrade. Anything rejected is logged and dropped.

use std::collections::HashMap;
use std::path::PathBuf;

use sha1::{Digest, Sha1};

use crate::env_str;

const MAGIC: &[u8; 4] = b"AGS1";
const NEWEST: &str = "state";
const OLDER: &str = "prev";
const DIGEST: usize = 20;

pub struct Store {
    saver: &'static str,
    dir: Option<PathBuf>,
    /// By file name, when there is no directory.
    mem: HashMap<String, Vec<u8>>,
}

impl Store {
    /// `SAVER_STATE_DIR`, if set, for `saver`'s states.
    pub fn new(saver: &'static str) -> Self {
        let dir = env_str(&["SAVER_STATE_DIR"], "");
        Self::at(saver, (!dir.is_empty()).then(|| PathBuf::from(dir)))
    }

    pub fn at(saver: &'static str, dir: Option<PathBuf>) -> Self {
        Self {
            saver,
            dir: dir.map(|d| d.join(saver)),
            mem: HashMap::new(),
        }
    }

    fn name(id: &str, which: &str) -> String {
        format!("{id}.{which}")
    }

    fn read(&self, name: &str) -> Option<Vec<u8>> {
        match &self.dir {
            Some(d) => std::fs::read(d.join(name)).ok(),
            None => self.mem.get(name).cloned(),
        }
    }

    fn exists(&self, name: &str) -> bool {
        match &self.dir {
            Some(d) => d.join(name).exists(),
            None => self.mem.contains_key(name),
        }
    }

    fn remove(&mut self, name: &str) {
        match &self.dir {
            Some(d) => {
                let _ = std::fs::remove_file(d.join(name));
            }
            None => {
                self.mem.remove(name);
            }
        }
    }

    /// Keep `payload` as ROM `id`'s newest state, the previous newest as
    /// its fallback.
    pub fn put(&mut self, id: &str, format: &str, payload: &[u8]) {
        let mut framed = Vec::with_capacity(payload.len() + 64);
        framed.extend_from_slice(MAGIC);
        framed.push(format.len() as u8);
        framed.extend_from_slice(format.as_bytes());
        framed.extend_from_slice(&Sha1::digest(payload));
        framed.extend_from_slice(payload);
        let (newest, older) = (Self::name(id, NEWEST), Self::name(id, OLDER));
        let Some(d) = &self.dir else {
            if let Some(old) = self.mem.remove(&newest) {
                self.mem.insert(older, old);
            }
            self.mem.insert(newest, framed);
            return;
        };
        let tmp = d.join(Self::name(id, "tmp"));
        let wrote = std::fs::create_dir_all(d)
            .and_then(|()| std::fs::write(&tmp, &framed))
            .and_then(|()| {
                if d.join(&newest).exists() {
                    std::fs::rename(d.join(&newest), d.join(&older))?;
                }
                std::fs::rename(&tmp, d.join(&newest))
            });
        if let Err(e) = wrote {
            eprintln!(
                "[screensaver] {}: saving state to {}: {e}",
                self.saver,
                d.display()
            );
        }
    }

    /// ROM `id`'s newest state in `format` that reads back whole; any that
    /// do not are dropped.
    pub fn get(&mut self, id: &str, format: &str) -> Option<Vec<u8>> {
        for which in [NEWEST, OLDER] {
            let name = Self::name(id, which);
            let Some(framed) = self.read(&name) else {
                continue;
            };
            match unframe(&framed, format) {
                Ok(at) => return Some(framed[at..].to_vec()),
                Err(why) => {
                    eprintln!("[screensaver] {}: state {name} {why}; dropped", self.saver);
                    self.remove(&name);
                }
            }
        }
        None
    }

    /// Drop ROM `id`'s newest state, so its fallback is the next one loaded.
    pub fn discard(&mut self, id: &str) {
        let newest = Self::name(id, NEWEST);
        if self.exists(&newest) {
            self.remove(&newest);
        } else {
            self.remove(&Self::name(id, OLDER));
        }
    }

    /// Drop every state of ROM `id`: its next boot is power-on.
    pub fn forget(&mut self, id: &str) {
        self.remove(&Self::name(id, NEWEST));
        self.remove(&Self::name(id, OLDER));
    }
}

/// Where the payload starts, or why the frame is not one `put` wrote in
/// `format`.
fn unframe(b: &[u8], format: &str) -> Result<usize, String> {
    if b.len() < MAGIC.len() + 1 || &b[..MAGIC.len()] != MAGIC {
        return Err("is not a save state".into());
    }
    let n = usize::from(b[MAGIC.len()]);
    let tag_at = MAGIC.len() + 1;
    let at = tag_at + n + DIGEST;
    if b.len() < at {
        return Err("is cut short".into());
    }
    let tag = &b[tag_at..tag_at + n];
    if tag != format.as_bytes() {
        let tag = String::from_utf8_lossy(tag);
        return Err(format!("is {tag}, not {format}"));
    }
    if Sha1::digest(&b[at..])[..] != b[at - DIGEST..at] {
        return Err("fails its checksum".into());
    }
    Ok(at)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("afterglow-state-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn newest_then_fallback_in_memory_and_on_disk() {
        let dir = scratch("fallback");
        for mut s in [Store::at("t", None), Store::at("t", Some(dir.clone()))] {
            assert_eq!(s.get("rom", "f1"), None);
            s.put("rom", "f1", b"one");
            s.put("rom", "f1", b"two");
            assert_eq!(s.get("rom", "f1").as_deref(), Some(&b"two"[..]));
            s.discard("rom");
            assert_eq!(s.get("rom", "f1").as_deref(), Some(&b"one"[..]));
            s.put("rom", "f1", b"three");
            s.forget("rom");
            assert_eq!(s.get("rom", "f1"), None);
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_corrupt_or_foreign_state_is_dropped_for_its_fallback() {
        let dir = scratch("corrupt");
        let mut s = Store::at("t", Some(dir.clone()));
        s.put("rom", "f1", b"good");
        s.put("rom", "f1", b"flipped");
        let file = dir.join("t").join("rom.state");
        let mut b = std::fs::read(&file).unwrap();
        *b.last_mut().unwrap() ^= 1;
        std::fs::write(&file, b).unwrap();
        assert_eq!(s.get("rom", "f1").as_deref(), Some(&b"good"[..]));
        assert!(!file.exists(), "the corrupt one is gone");
        assert_eq!(s.get("rom", "f2"), None, "another core's format");
        let _ = std::fs::remove_dir_all(dir);
    }
}
