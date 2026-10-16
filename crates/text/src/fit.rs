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
///
/// Callers must refuse vertical text, RTL text and multi-layer input before fitting; this solver
/// does not check for them.
pub fn fit_scale(engine: &mut crate::TextEngine, layer: &TextLayer, dpi: f32) -> FitResult {
    if !is_fitted(layer) {
        return UNTOUCHED;
    }
    let fallback = sane_fallback(layer);
    let sizes: Vec<f32> = layer.char_runs().iter().map(|r| sane(r.style.size_pt, fallback).max(MIN_PT)).collect();
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

/// The layer's own size when finite, else MIN_PT: what a non-finite size is replaced with.
fn sane_fallback(layer: &TextLayer) -> f32 {
    if layer.size_pt.is_finite() { layer.size_pt.clamp(MIN_PT, MAX_PT) } else { MIN_PT }
}

fn sane(v: f32, fallback: f32) -> f32 {
    if v.is_finite() { v } else { fallback }
}

/// Multiplies `size_pt` and every run's `size_pt` (and explicit `leading_pt`, `baseline_shift_pt`)
/// by `scale`, clamping each size to MIN_PT..=MAX_PT. Keeps run ratios except where clamped.
/// Paragraph indents and spacing scale too (signs kept, so hanging indents stay negative).
/// Non-finite sizes become the layer's size (or MIN_PT) first, so NaN never reaches layout.
pub fn apply_scale(layer: &mut TextLayer, scale: f32) {
    let scale = if scale.is_finite() { scale } else { 1.0 };
    let fallback = sane_fallback(layer);
    let size = |v: f32| (sane(v, fallback) * scale).clamp(MIN_PT, MAX_PT);
    let length = |v: f32| if v.is_finite() { (v * scale).clamp(-MAX_PT, MAX_PT) } else { 0.0 };
    layer.size_pt = size(layer.size_pt);
    for run in &mut layer.runs {
        let s = &mut run.style;
        s.size_pt = size(s.size_pt);
        s.leading_pt = s.leading_pt.filter(|v| v.is_finite()).map(size);
        s.baseline_shift_pt = length(s.baseline_shift_pt);
    }
    for para in &mut layer.paragraphs {
        let p = &mut para.style;
        p.first_line_indent_pt = length(p.first_line_indent_pt);
        p.start_indent_pt = length(p.start_indent_pt);
        p.end_indent_pt = length(p.end_indent_pt);
        p.space_before_pt = length(p.space_before_pt);
        p.space_after_pt = length(p.space_after_pt);
    }
}

/// fit_scale then apply_scale when `layer.fit` is Some and the shape is Box or Area. Returns the result.
///
/// Callers must refuse vertical text, RTL text and multi-layer input before calling.
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

    fn with_para(t: &mut TextLayer, p: photocraft_doc::text::ParagraphStyle) {
        t.paragraphs = vec![photocraft_doc::text::ParagraphRun { len: t.text.len(), style: p }];
    }

    #[test]
    fn indents_that_swallow_the_box_scale_down_and_still_fit() {
        let mut e = TextEngine::new();
        let mut t = fit_box("one two three four", 12.0, 100.0, 200.0);
        with_para(&mut t, photocraft_doc::text::ParagraphStyle { start_indent_pt: 60.0, end_indent_pt: 60.0, ..Default::default() });
        assert!(e.layout(&t, 72.0).overflowed, "unscaled indents leave no room");
        let r = refit(&mut e, &mut t, 72.0);
        assert!(r.fits && t.size_pt > 1.0, "{r:?} {}", t.size_pt);
        assert!(!e.layout(&t, 72.0).overflowed);
        assert!(t.paragraphs[0].style.start_indent_pt < 60.0);
    }

    #[test]
    fn apply_scale_halves_paragraph_fields_and_keeps_hanging_negative() {
        let mut t = fit_box("ab", 10.0, 100.0, 100.0);
        with_para(
            &mut t,
            photocraft_doc::text::ParagraphStyle {
                first_line_indent_pt: -8.0,
                start_indent_pt: 20.0,
                end_indent_pt: 10.0,
                space_before_pt: 6.0,
                space_after_pt: 4.0,
                ..Default::default()
            },
        );
        apply_scale(&mut t, 0.5);
        let p = &t.paragraphs[0].style;
        assert_eq!((p.first_line_indent_pt, p.start_indent_pt, p.end_indent_pt, p.space_before_pt, p.space_after_pt), (-4.0, 10.0, 5.0, 3.0, 2.0));
    }

    #[test]
    fn non_finite_sizes_never_reach_layout() {
        let mut e = TextEngine::new();
        let mut t = fit_box("ab", 10.0, 100.0, 100.0);
        set_run_sizes(&mut t, &[(1, f32::NAN), (1, f32::INFINITY)]);
        apply_scale(&mut t, 2.0);
        assert!(t.runs.iter().all(|r| r.style.size_pt.is_finite() && r.style.size_pt >= MIN_PT));
        assert_eq!(t.runs[0].style.size_pt, 20.0);
        let mut u = fit_box("ab", f32::NAN, 100.0, 100.0);
        set_run_sizes(&mut u, &[(1, f32::NAN), (1, 12.0)]);
        let r = refit(&mut e, &mut u, 72.0);
        assert!(r.scale.is_finite() && u.size_pt.is_finite() && u.runs.iter().all(|r| r.style.size_pt.is_finite()));
    }
}
