//! `/stream` under a picture saver's load: every cell of a doom-sized grid
//! changing every frame, faster than any viewer can take it.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::codec::apply;
use super::http::{handle, MAX_RATE, NEVER};
use super::Mirror;
use crate::grid::{Cell, Grid};
use crate::surface::Panel;

const COLS: usize = 960;
const ROWS: usize = 216;

/// A mirror describing pine's doom grid, served on an ephemeral port, with
/// one viewer's stream open and registered.
fn open() -> (Arc<Mirror>, BufReader<TcpStream>) {
    let m = Mirror::new(30);
    let panel = Panel::new(COLS * 2, ROWS * 5, COLS * 2);
    m.describe("test", &Grid::new(&panel, 2, 5), &panel, &[0, 0xFF]);
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    {
        let m = Arc::clone(&m);
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                let m = Arc::clone(&m);
                std::thread::spawn(move || handle(&m, s));
            }
        });
    }
    let mut s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    s.write_all(b"GET /stream HTTP/1.1\r\nHost: x\r\n\r\n")
        .unwrap();
    while !m.watched() {
        std::thread::yield_now();
    }
    let mut r = BufReader::new(s);
    let mut line = String::new();
    while line != "\r\n" {
        line.clear();
        r.read_line(&mut line).unwrap();
    }
    (m, r)
}

/// Publish a new all-changing frame every millisecond until `stop`, with the
/// frame's number in cell 0's glyph and a byte of noise per cell after it —
/// worse than doom compresses, so the byte cap is what binds.
fn publish(m: &Arc<Mirror>, stop: &Arc<AtomicBool>, latest: &Arc<AtomicU64>) -> JoinHandle<()> {
    let (m, stop, latest) = (Arc::clone(m), Arc::clone(stop), Arc::clone(latest));
    std::thread::spawn(move || {
        let mut r = 0x2545_F491u32;
        let pictures: Vec<Vec<Cell>> = (0..8)
            .map(|_| {
                (0..COLS * ROWS)
                    .map(|_| {
                        r ^= r << 13;
                        r ^= r >> 17;
                        r ^= r << 5;
                        Cell::new(3, (r & 0xFF) as u16)
                    })
                    .collect()
            })
            .collect();
        let mut n = 0u64;
        let mut cells = pictures[0].clone();
        while !stop.load(Ordering::Relaxed) {
            n += 1;
            cells.copy_from_slice(&pictures[n as usize % pictures.len()]);
            cells[0] = Cell::new(n as u16, 0);
            m.publish(&cells);
            latest.store(n, Ordering::Relaxed);
            std::thread::sleep(Duration::from_millis(1));
        }
    })
}

/// One chunk, which on a direct socket is exactly one record.
fn record(r: &mut BufReader<TcpStream>) -> Vec<u8> {
    let mut line = String::new();
    r.read_line(&mut line).unwrap();
    let n = usize::from_str_radix(line.trim(), 16).unwrap();
    let mut b = vec![0; n + 2];
    r.read_exact(&mut b).unwrap();
    b.truncate(n);
    b
}

/// Loopback takes hundreds of MB/s, so nothing but the cap stands between
/// this stream and the 34 MB/s doom used to send a browser.
#[test]
fn an_all_changing_grid_streams_at_the_byte_cap() {
    let (m, mut r) = open();
    let (stop, latest) = (Arc::new(AtomicBool::new(false)), Arc::default());
    let publisher = publish(&m, &stop, &latest);

    let mut have = vec![NEVER; COLS * ROWS];
    let (mut bytes, mut records, mut largest) = (0, 0, 0);
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_secs(2) {
        let rec = record(&mut r);
        apply(&mut have, &rec);
        (bytes, records, largest) = (bytes + rec.len(), records + 1, largest.max(rec.len()));
    }
    let secs = t0.elapsed().as_secs_f64();
    stop.store(true, Ordering::Relaxed);
    publisher.join().unwrap();

    let rate = bytes as f64 / secs;
    eprintln!(
        "{records} records, {:.0} KB/s, largest {largest} B",
        rate / 1e3
    );
    // One record past the budget: the cap sleeps AFTER a write.
    assert!(
        bytes as f64 <= MAX_RATE as f64 * secs + largest as f64,
        "{rate:.0} B/s over a {MAX_RATE} B/s cap"
    );
    assert!(records > 4, "it streamed: {records} records");
    assert!(
        have.iter().all(|c| *c != NEVER),
        "a keyframe covered the grid"
    );
}

/// A viewer that stops reading for a second while 1000 frames are published
/// must, once it reads again, be on the newest frame within a handful of
/// records: frames are dropped, never queued, so the server holds one frame
/// per viewer however far behind the browser is.
#[test]
fn a_stalled_viewer_resumes_on_the_newest_frame_not_a_backlog() {
    let (m, mut r) = open();
    let (stop, latest) = (
        Arc::new(AtomicBool::new(false)),
        Arc::<AtomicU64>::default(),
    );
    let publisher = publish(&m, &stop, &latest);

    let mut have = vec![NEVER; COLS * ROWS];
    apply(&mut have, &record(&mut r));
    std::thread::sleep(Duration::from_secs(1));
    let target = latest.load(Ordering::Relaxed) as u16;

    let t0 = Instant::now();
    let mut records = 0;
    while have[0].glyph() < target as usize {
        apply(&mut have, &record(&mut r));
        records += 1;
        assert!(
            t0.elapsed() < Duration::from_secs(2),
            "{records} records and still behind"
        );
    }
    stop.store(true, Ordering::Relaxed);
    publisher.join().unwrap();
    eprintln!("caught up in {records} records, {:?}", t0.elapsed());
    assert!(
        records < 30,
        "{records} records to reach frame {target}: a backlog"
    );
}
