//! Drawing on the processor (R27's commit 31, the owner's answer 106,
//! spike S4): the frame a window with no graphics card draws, measured
//! the way the CPU presenter draws it, with Vello's CPU renderer, the
//! whole frame each time.

#![allow(clippy::unwrap_used)]

use std::time::{Duration, Instant};

use crate::panel_list::list_synthetic;
use crate::rig::{Rig, app_on, fixture};

/// The median of some durations.
fn median(mut times: Vec<Duration>) -> Duration {
    times.sort();
    times[times.len() / 2]
}

/// Spike S4: a 1080p window whose left panel lists 100,000 rows, drawn
/// with Vello's CPU renderer as the CPU presenter draws it (a full frame
/// each time), with the copy into the presenter's 32-bit pixels timed
/// apart. A measurement, not a check: it runs when asked for
/// (`--run-ignored only`), in a release build for numbers worth
/// reporting.
#[test]
#[ignore = "spike S4: a measurement; run with --run-ignored only, in a release build"]
fn spike_s4_a_1080p_frame_of_a_hundred_thousand_rows_on_the_processor() {
    let dir = fixture(&[], &[]);
    let (app, runtime) = app_on(dir.path(), dir.path());
    let mut rig = Rig::sized(app, runtime, (1920, 1080));
    for blob in diroxide_gui::fonts::blobs() {
        rig.h.register_fonts(blob);
    }
    list_synthetic(&mut rig, dir.path(), 100_000);
    let started = Instant::now();
    let image = rig.h.render();
    let first = started.elapsed();
    assert_eq!((image.width(), image.height()), (1920, 1080));
    let mut frames = Vec::new();
    let mut copies = Vec::new();
    let mut pixels: Vec<u32> = vec![0; 1920 * 1080];
    for _ in 0..20 {
        rig.press("down");
        let at = Instant::now();
        let image = rig.h.render();
        frames.push(at.elapsed());
        let at = Instant::now();
        for (out, px) in pixels.iter_mut().zip(image.pixels()) {
            let [r, g, b, _] = px.0;
            *out = (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b);
        }
        copies.push(at.elapsed());
    }
    rig.press("end");
    let at = Instant::now();
    let _ = rig.h.render();
    let end = at.elapsed();
    let frame = median(frames.clone());
    let worst = frames.iter().max().copied().unwrap_or_default();
    let copy = median(copies);
    eprintln!(
        "S4: 1920x1080, 100,000 rows; first frame {first:?}; a frame after Down, median {frame:?}, worst {worst:?}; after End {end:?}; the copy to 32-bit pixels, median {copy:?}"
    );
    assert!(
        pixels.iter().any(|p| *p != pixels[0]),
        "something was drawn"
    );
}
