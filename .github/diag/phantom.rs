//! TEMPORARY DIAGNOSTICS: does a listener stay reachable after it is dropped while another
//! thread of the same process starts child processes?

use std::net::{SocketAddr, TcpListener, TcpStream};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_millis(300);

fn run(spawning: bool, rounds: u32, first_port: u16) {
    let stop = Arc::new(AtomicBool::new(false));
    let spawner = spawning.then(|| {
        let stop = stop.clone();
        std::thread::spawn(move || {
            let mut started = 0u32;
            while !stop.load(Ordering::Relaxed) {
                let _ = Command::new("/usr/bin/true").status();
                started += 1;
            }
            started
        })
    });
    let (mut reached, mut skipped) = (0u32, 0u32);
    let mut longest = Duration::ZERO;
    for i in 0..rounds {
        let addr = SocketAddr::from(([127, 0, 0, 1], first_port + (i % 2000) as u16));
        match TcpListener::bind(addr) {
            Ok(listener) => drop(listener),
            Err(_) => {
                skipped += 1;
                continue;
            }
        }
        if TcpStream::connect_timeout(&addr, TIMEOUT).is_ok() {
            reached += 1;
            let since = Instant::now();
            while TcpStream::connect_timeout(&addr, TIMEOUT).is_ok()
                && since.elapsed() < Duration::from_secs(2)
            {}
            longest = longest.max(since.elapsed());
        }
    }
    stop.store(true, Ordering::Relaxed);
    let started = spawner.map(|h| h.join().unwrap());
    println!(
        "another thread starting processes: {spawning} ({started:?} started) | rounds: {rounds} | \
         connects that reached the dropped listener: {reached} | longest it stayed reachable: \
         {longest:?} | ports skipped: {skipped}"
    );
}

fn main() {
    run(false, 20_000, 20_000);
    run(true, 20_000, 24_000);
    run(false, 20_000, 28_000);
}
