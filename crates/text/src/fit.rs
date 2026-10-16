//! Dynamic Text: finds the largest uniform scale of a layer's type sizes at which its text still
//! fits its box or area shape. The only "doesn't fit" signal is `TextLayout::overflowed`.

use photocraft_doc::TextLayer;
use photocraft_doc::text::TextShape;

pub const MIN_PT: f32 = 0.1;
pub const MAX_PT: f32 = 1296.0;
pub const MAX_PROBES: usize = 20;

/// The search stops once the bracket is this tight (ratio of upper to lower scale).
const TOLERANCE: f32 = 1.001;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FitResult {
    pub scale: f32,
    pub fits: bool,
    pub probes: usize,
}

const UNTOUCHED: FitResult = FitResult { scale: 1.0, fits: true, probes: 0 };

fn is_fitted(layer: &TextLayer) -> bool {
    layer.fit.is_some() && !layer.text.is_empty() && matches!(layer.shape, TextShape::Box { .. } | TextShape::Area { .. })
}

/// Largest uniform scale of every run's size (and `size_pt`) at which `layer` lays out with no
/// overflow, the sizes staying within MIN_PT..=MAX_PT. Bounded by MAX_PROBES layouts.
/// Point/Path layers, fit None, or empty text: scale 1, fits true, 0 probes.
pub fn fit_scale(engine: &mut crate::TextEngine, layer: &TextLayer, dpi: f32) -> FitResult {
    if !is_fitted(layer) {
        return UNTOUCHED;
    }
    let sizes: Vec<f32> = layer.char_runs().iter().map(|r| r.style.size_pt.max(MIN_PT)).collect();
    let max_run = sizes.iter().copied().fold(MIN_PT, f32::max);
    let min_run = sizes.iter().copied().fold(MAX_PT, f32::min);
    let (lo, hi) = (MIN_PT / max_run, MAX_PT / min_run);

    let probe_count = std::cell::Cell::new(0);
    let probe = |engine: &mut crate::TextEngine, scale: f32| {
        probe_count.set(probe_count.get() + 1);
        let mut clone = layer.clone();
        clone.cache = None;
        clone.psd_raw = None;
        apply_scale(&mut clone, scale);
        !engine.layout(&clone, dpi).overflowed
    };

    // Warm start at the current size, then bisect in log space between the largest scale known
    // to fit (`low`) and the smallest known to overflow (`high`).
    let start = 1.0_f32.clamp(lo, hi);
    let mut best = None;
    let (mut low, mut high) = (lo, hi);
    if probe(engine, start) {
        best = Some(start);
        low = start;
    } else {
        high = start;
    }
    while high / low >= TOLERANCE && probe_count.get() < MAX_PROBES {
        let mid = (low * high).sqrt();
        if probe(engine, mid) {
            best = Some(mid);
            low = mid;
        } else {
            high = mid;
        }
    }
    if best.is_none() && probe_count.get() < MAX_PROBES && probe(engine, lo) {
        best = Some(lo);
    }
    let probes = probe_count.get();
    match best {
        Some(scale) => FitResult { scale, fits: true, probes },
        None => FitResult { scale: lo, fits: false, probes },
    }
}

/// Multiplies `size_pt` and every run's `size_pt` (and explicit `leading_pt`, `baseline_shift_pt`)
/// by `scale`, clamping each size to MIN_PT..=MAX_PT. Keeps run ratios except where clamped.
pub fn apply_scale(layer: &mut TextLayer, scale: f32) {
    let size = |v: f32| (v * scale).clamp(MIN_PT, MAX_PT);
    layer.size_pt = size(layer.size_pt);
    for run in &mut layer.runs {
        let s = &mut run.style;
        s.size_pt = size(s.size_pt);
        s.leading_pt = s.leading_pt.map(size);
        s.baseline_shift_pt *= scale;
    }
}

/// fit_scale then apply_scale when `layer.fit` is Some and the shape is Box or Area. Returns the result.
pub fn refit(engine: &mut crate::TextEngine, layer: &mut TextLayer, dpi: f32) -> FitResult {
    let result = fit_scale(engine, layer, dpi);
    if is_fitted(layer) {
        apply_scale(layer, result.scale);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TextEngine;
    use crate::tests::{area_ellipse_layer, box_layer};
    use photocraft_doc::text::{TextFit, TextRun};

    fn fit_box(text: &str, size: f32, w: f32, h: f32) -> TextLayer {
        let mut t = box_layer(text, size, (0.0, 0.0, w, h));
        t.fit = Some(TextFit::default());
        t
    }

    /// Splits the layer's text into runs of the given (byte length, size) pairs.
    fn set_run_sizes(t: &mut TextLayer, parts: &[(usize, f32)]) {
        t.runs = parts
            .iter()
            .map(|&(len, size_pt)| {
                let mut style = t.char_runs()[0].style.clone();
                style.size_pt = size_pt;
                TextRun { len, style }
            })
            .collect();
    }

    #[test]
    fn fitted_box_fits_and_slightly_larger_does_not() {
        let mut e = TextEngine::new();
        let mut t = fit_box("The quick brown fox jumps over the lazy dog", 12.0, 300.0, 120.0);
        let r = refit(&mut e, &mut t, 72.0);
        println!("probes fitted_box: {}", r.probes);
        assert!(r.fits && r.probes <= MAX_PROBES);
        assert!(!e.layout(&t, 72.0).overflowed);
        let mut bigger = t.clone();
        apply_scale(&mut bigger, 1.02);
        assert!(e.layout(&bigger, 72.0).overflowed, "fit should be tight: {}", t.size_pt);
    }

    #[test]
    fn wider_box_gives_bigger_text_and_fewer_lines() {
        let mut e = TextEngine::new();
        let text = "one two three four five six seven eight";
        let mut narrow = fit_box(text, 12.0, 150.0, 300.0);
        let mut wide = fit_box(text, 12.0, 600.0, 300.0);
        refit(&mut e, &mut narrow, 72.0);
        refit(&mut e, &mut wide, 72.0);
        assert!(wide.size_pt > narrow.size_pt);
        assert!(e.layout(&wide, 72.0).lines.len() < e.layout(&narrow, 72.0).lines.len());
    }

    #[test]
    fn long_text_bottoms_out_at_minimum() {
        let mut e = TextEngine::new();
        let mut t = fit_box(&"word ".repeat(5000), 12.0, 2.0, 2.0);
        let r = refit(&mut e, &mut t, 72.0);
        println!("probes long_text: {}", r.probes);
        assert!(!r.fits && r.probes <= MAX_PROBES);
        assert!((t.size_pt - MIN_PT).abs() < 1e-4);
    }

    #[test]
    fn mixed_sizes_keep_their_ratio() {
        let mut e = TextEngine::new();
        let mut t = fit_box("big small", 12.0, 400.0, 200.0);
        set_run_sizes(&mut t, &[(3, 30.0), (6, 10.0)]); // "big" 30pt, " small" 10pt
        refit(&mut e, &mut t, 72.0);
        let s: Vec<f32> = t.runs.iter().map(|r| r.style.size_pt).collect();
        assert!((s[0] / s[1] - 3.0).abs() < 1e-3, "{s:?}");
    }

    #[test]
    fn area_ellipse_fits_tightly() {
        let mut e = TextEngine::new();
        let mut t = area_ellipse_layer(&"talk ".repeat(12), 12.0, 260.0, 160.0);
        t.fit = Some(TextFit::default());
        let r = refit(&mut e, &mut t, 72.0);
        println!("probes area: {}", r.probes);
        assert!(r.fits && !e.layout(&t, 72.0).overflowed);
        let mut bigger = t.clone();
        apply_scale(&mut bigger, 1.02);
        assert!(e.layout(&bigger, 72.0).overflowed);
    }

    #[test]
    fn not_fitted_point_or_empty_is_untouched() {
        let mut e = TextEngine::new();
        let mut t = fit_box("", 12.0, 100.0, 100.0);
        assert_eq!(refit(&mut e, &mut t, 72.0), FitResult { scale: 1.0, fits: true, probes: 0 });
        let mut p = fit_box("hello", 12.0, 100.0, 100.0);
        p.shape = TextShape::Point;
        refit(&mut e, &mut p, 72.0);
        assert_eq!(p.size_pt, 12.0);
        let mut n = fit_box("hello", 12.0, 100.0, 100.0);
        n.fit = None;
        refit(&mut e, &mut n, 72.0);
        assert_eq!(n.size_pt, 12.0);
    }

    #[test]
    fn apply_scale_scales_leading_and_shift_and_clamps() {
        let mut t = fit_box("ab", 10.0, 100.0, 100.0);
        set_run_sizes(&mut t, &[(1, 10.0), (1, 1000.0)]);
        t.runs[0].style.leading_pt = Some(20.0);
        t.runs[0].style.baseline_shift_pt = 4.0;
        apply_scale(&mut t, 2.0);
        assert_eq!(t.runs[0].style.size_pt, 20.0);
        assert_eq!(t.runs[0].style.leading_pt, Some(40.0));
        assert_eq!(t.runs[0].style.baseline_shift_pt, 8.0);
        assert_eq!(t.runs[1].style.size_pt, MAX_PT);
        apply_scale(&mut t, 1e-9);
        assert_eq!(t.runs[0].style.size_pt, MIN_PT);
    }
}
