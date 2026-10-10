//! The `mac-engine` process and the one thread that talks to it.
//!
//! The thread starts the engine on first use, runs the pilot against every
//! screen it sends, paces it at the Mac's sixty a second, and parks it (the
//! engine blocks on its pipe) whenever no `battlechess` saver is showing. The
//! render thread only ever `try_lock`s the last finished screen.
//!
//! An engine that dies is started again with the Mac booting afresh; one that
//! dies within seconds of starting, three times running, is given up on until
//! the next claim.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use super::pilot::{Phase, Pilot};
use super::proto::{blank, CHANGED, FRAME};
use crate::engine_slot::{self, Slot};

/// The Mac Plus's vertical retrace, 60.1474 Hz.
const TICK: Duration = Duration::from_nanos(16_625_800);
const EARLY: Duration = Duration::from_secs(20);
const GIVE_UP: u32 = 3;
const BACKOFF: Duration = Duration::from_millis(if cfg!(test) { 20 } else { 2000 });
/// Off in tests: the Mac runs as fast as the host does.
const PACED: bool = !cfg!(test);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Want {
    pub engine: PathBuf,
    pub rom: PathBuf,
    /// Boot disk first.
    pub disks: Vec<PathBuf>,
    /// The game disk's name, for the Finder's type-to-select.
    pub volume: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Starting = 0,
    Running = 1,
    /// Would not start; waits for different files.
    Failed = 2,
}

struct Frame {
    bits: Box<[u8; FRAME]>,
    seq: u32,
}

/// A claim's `rev` bump is also a failed engine's retry.
pub struct Engine {
    slot: Slot<Want, Frame>,
    state: AtomicU8,
    /// The pilot's phase, for tests to wait on.
    phase: AtomicU8,
}

impl Engine {
    /// The process's engine, its thread started on first use.
    pub fn get() -> &'static Self {
        static E: OnceLock<&'static Engine> = OnceLock::new();
        E.get_or_init(Self::spawn)
    }

    /// An engine of its own, for tests that must not share the process's.
    pub fn spawn() -> &'static Self {
        let e: &'static Self = Box::leak(Box::new(Self {
            slot: Slot::new(
                Frame {
                    bits: blank(),
                    seq: 0,
                },
                (),
            ),
            state: AtomicU8::new(State::Starting as u8),
            phase: AtomicU8::new(Phase::Boot as u8),
        }));
        std::thread::Builder::new()
            .name("mac".into())
            .spawn(move || run(e))
            .expect("spawning the mac thread");
        e
    }

    #[cfg(test)]
    pub fn playing(&self) -> bool {
        self.phase.load(Ordering::Relaxed) == Phase::Play as u8
    }

    pub fn state(&self) -> State {
        match self.state.load(Ordering::Relaxed) {
            0 => State::Starting,
            1 => State::Running,
            _ => State::Failed,
        }
    }

    /// The screen, into `out`, if it is newer than `seen`; its sequence
    /// number. Never waits: a screen being written is a frame late, not a
    /// stall.
    pub fn latest(&self, seen: u32, out: &mut [u8; FRAME]) -> Option<u32> {
        let f = self.slot.peek()?;
        (f.seq != seen).then(|| {
            out.copy_from_slice(&f.bits[..]);
            f.seq
        })
    }

    fn publish(&self, bits: &[u8; FRAME]) {
        let mut f = self.slot.frame();
        f.bits.copy_from_slice(bits);
        f.seq = f.seq.wrapping_add(1).max(1);
        self.state.store(State::Running as u8, Ordering::Relaxed);
    }
}

impl engine_slot::Engine for Engine {
    type Want = Want;

    fn claim(&self, id: u64, want: Want) {
        self.slot.claim(id, want);
    }

    fn release(&self, id: u64) {
        self.slot.release(id);
    }
}

struct Session {
    child: Child,
    input: ChildStdin,
    output: ChildStdout,
    want: Want,
    pilot: Pilot,
    screen: Box<[u8; FRAME]>,
    started: Instant,
    next: Instant,
}

impl Session {
    fn start(want: &Want) -> std::io::Result<Self> {
        let mut child = Command::new(&want.engine)
            .arg(&want.rom)
            .args(&want.disks)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;
        let (Some(input), Some(output)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::other("no pipes"));
        };
        Ok(Self {
            child,
            input,
            output,
            want: want.clone(),
            pilot: Pilot::new(&want.volume),
            screen: blank(),
            started: Instant::now(),
            next: Instant::now(),
        })
    }

    /// One sixtieth: the engine's screen in, the pilot's command out.
    fn tick(&mut self, e: &Engine) -> std::io::Result<()> {
        let mut tag = [0];
        self.output.read_exact(&mut tag)?;
        let changed = tag[0] == CHANGED;
        if changed {
            self.output.read_exact(&mut self.screen[..])?;
            e.publish(&self.screen);
        }
        let cmd = self.pilot.step(&self.screen, changed);
        let phase = self.pilot.phase() as u8;
        if e.phase.swap(phase, Ordering::Relaxed) != phase || cmd.reset {
            eprintln!(
                "[screensaver] battlechess: {:?}{}",
                self.pilot.phase(),
                if cmd.reset { " (reset)" } else { "" }
            );
        }
        if PACED {
            let now = Instant::now();
            if self.next > now {
                std::thread::sleep(self.next - now);
            } else if now - self.next > TICK * 8 {
                self.next = now;
            }
            self.next += TICK;
        }
        self.input.write_all(&cmd.encode())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn run(e: &'static Engine) {
    let mut session: Option<Session> = None;
    let mut want: Option<Want> = None;
    let (mut rev, mut early, mut given_up) = (0, 0, false);
    loop {
        // The lock only for the claim's bookkeeping: the files are cloned
        // when a claim is new, not every sixtieth.
        let parked = {
            let (c, waited) = e.slot.wait(|c| given_up && c.rev == rev);
            let parked = waited || given_up;
            given_up = false;
            c.take(&mut rev, &mut want);
            parked
        };
        let want = want.as_ref().expect("set by every claim");
        if session.as_ref().is_some_and(|s| s.want != *want) {
            session = None;
            early = 0;
            e.state.store(State::Starting as u8, Ordering::Relaxed);
        }
        let s = match &mut session {
            Some(s) => s,
            None if early >= GIVE_UP => {
                e.state.store(State::Failed as u8, Ordering::Relaxed);
                eprintln!(
                    "[screensaver] battlechess: the Mac would not start; waiting for a new claim"
                );
                given_up = true;
                early = 0;
                continue;
            }
            None => match Session::start(want) {
                Ok(s) => session.insert(s),
                Err(err) => {
                    eprintln!(
                        "[screensaver] battlechess: {}: {err}",
                        want.engine.display()
                    );
                    early += 1;
                    std::thread::sleep(BACKOFF);
                    continue;
                }
            },
        };
        if parked {
            s.next = Instant::now();
        }
        if s.tick(e).is_err() {
            early = if s.started.elapsed() < EARLY {
                early + 1
            } else {
                0
            };
            eprintln!("[screensaver] battlechess: the Mac stopped; starting it again");
            session = None;
            std::thread::sleep(BACKOFF);
        }
    }
}
