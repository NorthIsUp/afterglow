//! The `mac-engine` process and the one thread that talks to it.
//!
//! The thread starts the engine on first use, runs the pilot against every
//! screen it sends, paces it at the Mac's sixty a second, and parks it (the
//! engine blocks on its pipe) whenever no `battlechess` saver is showing. The
//! render thread only ever `try_lock`s the last finished screen.
//!
//! An engine that dies is started again with the Mac booting afresh; one that
//! dies within seconds of starting, three times running, is given up on until
//! the saver asks for different files.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use super::pilot::{Phase, Pilot};
use super::proto::{blank, CHANGED, FRAME};

/// The Mac Plus's vertical retrace, 60.1474 Hz.
const TICK: Duration = Duration::from_nanos(16_625_800);
const EARLY: Duration = Duration::from_secs(20);
const GIVE_UP: u32 = 3;
const BACKOFF: Duration = Duration::from_millis(if cfg!(test) { 20 } else { 2000 });

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Want {
    pub engine: PathBuf,
    pub rom: PathBuf,
    /// Boot disk first.
    pub disks: Vec<PathBuf>,
    /// The game disk's name, for the Finder's type-to-select.
    pub volume: String,
    /// Off in tests: the Mac runs as fast as the host does.
    pub paced: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Starting = 0,
    Running = 1,
    /// Would not start; waits for different files.
    Failed = 2,
}

struct Ctl {
    /// The saver the engine runs for; 0 parks it.
    owner: u64,
    want: Option<Want>,
    /// Bumped per claim, so a failed engine tries again for new files.
    rev: u64,
}

struct Frame {
    bits: Box<[u8; FRAME]>,
    seq: u32,
}

pub struct Engine {
    ctl: Mutex<Ctl>,
    wake: Condvar,
    frame: Mutex<Frame>,
    state: AtomicU8,
    /// The pilot's phase, for tests to wait on.
    phase: AtomicU8,
}

/// A poisoned lock means a panic on the other thread; the data is still a
/// screen or a claim, so carry on with it.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
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
            ctl: Mutex::new(Ctl {
                owner: 0,
                want: None,
                rev: 0,
            }),
            wake: Condvar::new(),
            frame: Mutex::new(Frame {
                bits: blank(),
                seq: 0,
            }),
            state: AtomicU8::new(State::Starting as u8),
            phase: AtomicU8::new(Phase::Boot as u8),
        }));
        std::thread::Builder::new()
            .name("mac".into())
            .spawn(move || run(e))
            .expect("spawning the mac thread");
        e
    }

    pub fn claim(&self, id: u64, want: Want) {
        let mut c = lock(&self.ctl);
        c.owner = id;
        c.rev += 1;
        c.want = Some(want);
        self.wake.notify_one();
    }

    /// Park the engine, unless another saver has claimed it since.
    pub fn release(&self, id: u64) {
        let mut c = lock(&self.ctl);
        if c.owner == id {
            c.owner = 0;
        }
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
        let f = self.frame.try_lock().ok()?;
        (f.seq != seen).then(|| {
            out.copy_from_slice(&f.bits[..]);
            f.seq
        })
    }

    fn publish(&self, bits: &[u8; FRAME]) {
        let mut f = lock(&self.frame);
        f.bits.copy_from_slice(bits);
        f.seq = f.seq.wrapping_add(1).max(1);
        self.state.store(State::Running as u8, Ordering::Relaxed);
    }

    /// Blocks until a saver owns the engine; its files and claim number.
    fn wanted(&self, failed_rev: Option<u64>) -> (Want, u64) {
        let mut c = lock(&self.ctl);
        loop {
            if c.owner != 0 && failed_rev != Some(c.rev) {
                if let Some(w) = &c.want {
                    return (w.clone(), c.rev);
                }
            }
            c = self.wake.wait(c).unwrap_or_else(PoisonError::into_inner);
        }
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
        if self.want.paced {
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
    let mut early = 0;
    let mut failed_rev = None;
    let mut parked = false;
    loop {
        if lock(&e.ctl).owner == 0 {
            parked = true;
        }
        let (want, rev) = e.wanted(failed_rev);
        failed_rev = None;
        if session.as_ref().is_some_and(|s| s.want != want) {
            session = None;
            early = 0;
            e.state.store(State::Starting as u8, Ordering::Relaxed);
        }
        let s = match &mut session {
            Some(s) => s,
            None if early >= GIVE_UP => {
                e.state.store(State::Failed as u8, Ordering::Relaxed);
                eprintln!(
                    "[screensaver] battlechess: the Mac would not start; waiting for new files"
                );
                failed_rev = Some(rev);
                early = 0;
                continue;
            }
            None => match Session::start(&want) {
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
        if std::mem::take(&mut parked) {
            s.next = Instant::now();
        }
        if s.tick(e).is_err() {
            let quick = s.started.elapsed() < EARLY;
            early = if quick { early + 1 } else { 1 };
            eprintln!("[screensaver] battlechess: the Mac stopped; starting it again");
            session = None;
            std::thread::sleep(BACKOFF);
        }
    }
}
