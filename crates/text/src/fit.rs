//! Dynamic Text: finds the largest uniform scale of a layer's type sizes at which its text still
//! fits its box or area shape. The only "doesn't fit" signal is `TextLayout::overflowed`.

use photocraft_doc::TextLayer;
use photocraft_doc::text::{FitLine, TextFit, TextRun, TextShape};

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
/// The scale stops where the largest run reaches MAX_PT, so at the top end every run keeps its
/// ratio to the others. At the bottom end the search goes down until the largest run is at
/// MIN_PT, so the smaller runs clamp at MIN_PT before it: text too long for its shape loses its
/// size ratios there. That loss can't be avoided, since sizes below MIN_PT don't exist and
/// stopping earlier would leave the text overflowing.
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
    let (lo, hi) = (MIN_PT / max_run, MAX_PT / max_run);

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

/// Replaces only invalid (non-finite) sizes and lengths, as [`apply_scale`] does: sizes become
/// the layer's size (or MIN_PT), a leading becomes auto, shifts, indents and spacing become 0.
/// Every finite value, the user's base sizes included, is left as it is.
fn replace_non_finite(layer: &mut TextLayer) {
    let fallback = sane_fallback(layer);
    let length = |v: &mut f32| {
        if !v.is_finite() {
            *v = 0.0;
        }
    };
    layer.size_pt = sane(layer.size_pt, fallback);
    for run in &mut layer.runs {
        let s = &mut run.style;
        s.size_pt = sane(s.size_pt, fallback);
        s.leading_pt = s.leading_pt.filter(|v| v.is_finite());
        length(&mut s.baseline_shift_pt);
    }
    for para in &mut layer.paragraphs {
        let p = &mut para.style;
        for v in [&mut p.first_line_indent_pt, &mut p.start_indent_pt, &mut p.end_indent_pt, &mut p.space_before_pt, &mut p.space_after_pt] {
            length(v);
        }
    }
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

/// fit_scale then apply_scale for Area text; for Box text, [`solve_box`] written to
/// `fit.lines`, the sizes left alone (scale 1). Point/Path, fit None or empty text: untouched.
///
/// Callers must refuse vertical text, RTL text and multi-layer input before calling.
pub fn refit(engine: &mut crate::TextEngine, layer: &mut TextLayer, dpi: f32) -> FitResult {
    if !is_fitted(layer) {
        return UNTOUCHED;
    }
    if matches!(layer.shape, TextShape::Box { .. }) {
        replace_non_finite(layer);
        let solved = solve(engine, layer, dpi);
        if let Some(f) = layer.fit.as_mut() {
            f.lines = solved.lines;
        }
        return FitResult { scale: 1.0, fits: solved.fits, probes: solved.probes };
    }
    let result = fit_scale(engine, layer, dpi);
    apply_scale(layer, result.scale);
    result
}

/// Whether `layer` is Dynamic Text box type with solved lines: what [`bake`] turns into plain type.
pub fn is_fitted_box(layer: &TextLayer) -> bool {
    layer.fit.as_ref().is_some_and(|f| !f.valid_lines(&layer.text).is_empty()) && matches!(layer.shape, TextShape::Box { .. })
}

/// Whether every glyph of `b` is within 0.5 px of the same glyph of `a`, in order. A fitted
/// layout shapes each solved line alone and has an invisible direction mark per line (and the
/// spaces a break replaced) that the baked layout lacks: those are skipped, at most two per line
/// of `a`, so a baked text that lost a word does not pass.
fn same_glyphs(a: &crate::TextLayout, b: &crate::TextLayout) -> bool {
    if a.glyphs.len().saturating_sub(b.glyphs.len()) > 2 * a.lines.len().max(1) {
        return false;
    }
    let mut from = 0;
    for q in &b.glyphs {
        let found = a.glyphs.get(from..).and_then(|rest| rest.iter().position(|p| p.id == q.id && (p.x - q.x).abs() <= 0.5 && (p.y - q.y).abs() <= 0.5));
        let Some(off) = found else { return false };
        from += off + 1;
    }
    true
}

/// Line shrink factors `bake` tries in turn. A larger shrink moves a wide line's far glyphs
/// more than half a pixel, which the verification refuses anyway.
const MARGINS: [f32; 2] = [1.0 - 1e-5, 0.999];

/// Why [`bake`] cannot keep the fitted look as ordinary type.
#[derive(Clone, Debug, PartialEq)]
pub enum BakeError {
    /// A run is condensed (Horizontal Scale below 100%). Box wrapping measures the unscaled
    /// width, so the baked lines would wrap early.
    Condensed,
    /// The baked text lays out in a different number of lines than the fitted look.
    LineCount { baked: usize, fitted: usize },
    /// The baked text keeps the lines, but glyphs move more than 0.5 px.
    Drift,
    /// The text cannot be cut at a byte range.
    Cut { start: usize, end: usize },
}

impl std::fmt::Display for BakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Condensed => f.write_str("condensed type (Horizontal Scale below 100%) can't be kept yet"),
            Self::LineCount { baked, fitted } => write!(f, "the baked text lays out in {baked} lines, not the fitted {fitted}"),
            Self::Drift => f.write_str("the baked text lays out with glyphs more than 0.5 px from the fitted look"),
            Self::Cut { start, end } => write!(f, "the text cannot be cut at byte {start}..{end}"),
        }
    }
}

impl std::error::Error for BakeError {}

/// Whether a run of `layer`'s text is condensed (Horizontal Scale below 100%), which [`bake`]
/// refuses. Non-positive and non-finite scales lay out at 100%, so they don't count.
pub fn has_condensed_type(layer: &TextLayer) -> bool {
    layer.char_runs().iter().any(|r| r.len > 0 && r.style.horizontal_scale > 0.0 && r.style.horizontal_scale < 1.0)
}

/// Bakes a solved Dynamic Text box into ordinary type: each line's runs get their size times the
/// line's scale (leading and baseline shift follow, as layout does), and a forced line break
/// takes the place of the space each line ends with (or is inserted where the line follows no
/// space). Wrapped text counts a trailing space against the box, and the solved lines span it
/// exactly, so keeping the space would wrap a word early; the break replaces it and the runs
/// stay plain. The line gap becomes each line's explicit leading.
///
/// Known limitation: the word gap becomes extra tracking on each space inside a line (read from
/// the difference in that space's measured advance), an approximation of word spacing: it also
/// shows if text is typed in front of that space.
///
/// The result has no `fit` and lays out like the fitted layer (glyphs within 0.5 px); this is
/// what Release and PSD export use. It is verified: when the baked layout does not have the
/// fitted layout's lines, or the text cannot be cut cleanly, the result is an error naming the
/// cause and nothing is returned. When every margin fails and one of them changed the line
/// count, that is the cause reported: it is what a later margin's drift came from.
///
/// Condensed type (a run with Horizontal Scale below 100%) is refused before baking with
/// [`BakeError::Condensed`]: box wrapping measures its unscaled width.
///
/// Anything that is not a fitted box with solved lines (Area text, no lines yet) only loses its
/// `fit`: Area text already holds its fitted sizes. `engine` measures the layout.
pub fn bake(engine: &mut crate::TextEngine, layer: &TextLayer, dpi: f32) -> Result<TextLayer, BakeError> {
    use crate::layout::{FORCED_LINE_BREAK, scaled_style};
    use std::collections::{BTreeMap, BTreeSet};

    let mut out = layer.clone();
    let lines: Vec<FitLine> = match &layer.fit {
        Some(f) if matches!(layer.shape, TextShape::Box { .. }) => f.valid_lines(&layer.text).to_vec(),
        _ => Vec::new(),
    };
    out.fit = None;
    if lines.is_empty() || layer.text.is_empty() {
        return Ok(out);
    }
    if has_condensed_type(layer) {
        return Err(BakeError::Condensed);
    }
    replace_non_finite(&mut out);
    let text = &layer.text;
    let k = if dpi.is_finite() && dpi > 0.0 { dpi / 72.0 } else { 1.0 };
    let (word_gap, line_gap) = layer.fit.as_ref().map_or((0.0, 0.0), TextFit::gaps);

    // Where each line break goes: the spaces before a line start (possibly none) become one
    // forced break, unless the line follows a break already (or nothing).
    let mut breaks: BTreeMap<usize, usize> = BTreeMap::new();
    for l in lines.iter().skip(1) {
        let p = l.start as usize;
        let mut q = p;
        while q > 0 && text.as_bytes().get(q - 1) == Some(&b' ') {
            q -= 1;
        }
        if p == 0 || p >= text.len() || text.get(..q).is_none_or(|s| s.is_empty() || s.ends_with(['\r', '\n', FORCED_LINE_BREAK])) {
            continue;
        }
        breaks.insert(q, p);
    }

    // The fitted layout, and the same without the word gap: the difference of a space advance is
    // what the gap added.
    let fitted = engine.layout(layer, dpi);
    let mut space_extra: BTreeMap<usize, (usize, f32)> = BTreeMap::new();
    if word_gap != 0.0 {
        let mut plain_layer = layer.clone();
        plain_layer.cache = None;
        plain_layer.psd_raw = None;
        if let Some(f) = plain_layer.fit.as_mut() {
            f.word_gap = 0.0;
        }
        let plain = engine.layout(&plain_layer, dpi);
        let by_start: std::collections::HashMap<usize, f32> = plain.clusters.iter().map(|c| (c.range.start, c.advance)).collect();
        for c in fitted.clusters.iter().filter(|c| matches!(text.get(c.range.clone()), Some(" " | "\u{a0}"))) {
            // A space that a break replaces is gone: no tracking for it.
            if breaks.iter().any(|(&q, &p)| c.range.start >= q && c.range.start < p) {
                continue;
            }
            if let Some(delta) = by_start.get(&c.range.start).map(|p| c.advance - p).filter(|d| d.is_finite() && d.abs() > 1e-4) {
                space_extra.insert(c.range.start, (c.range.end, delta));
            }
        }
    }

    // Pieces: cut at run edges, line starts, break ranges and the spaces that get extra tracking.
    let runs = out.char_runs();
    let mut cuts: BTreeSet<usize> = BTreeSet::from([0, text.len()]);
    let mut at = 0usize;
    for r in &runs {
        at = at.saturating_add(r.len).min(text.len());
        cuts.insert(at);
    }
    cuts.extend(lines.iter().map(|l| l.start as usize).filter(|&p| p <= text.len()));
    for (&q, &p) in &breaks {
        cuts.extend([q, p]);
    }
    for (&a, &(z, _)) in &space_extra {
        cuts.extend([a, z]);
    }
    // A last empty paragraph (the text ends with a paragraph break) has no character of its own
    // and takes the style of the break before it: that break gets the empty line's own style.
    let break_len = if text.ends_with("\r\n") { 2 } else { 1 };
    let terminal = (lines.last().is_some_and(|l| l.start as usize >= text.len()) && text.ends_with(['\r', '\n'])).then(|| text.len().saturating_sub(break_len));
    cuts.extend(terminal);
    let cuts: Vec<usize> = cuts.into_iter().collect();
    let mut run_ends = Vec::with_capacity(runs.len());
    let mut at = 0usize;
    for r in &runs {
        at = at.saturating_add(r.len);
        run_ends.push(at);
    }
    let run_at = |off: usize| run_ends.partition_point(|&e| e <= off).min(runs.len().saturating_sub(1));
    let line_at = |off: usize| match terminal {
        Some(t) if off >= t => lines.len().saturating_sub(1),
        _ => lines.partition_point(|l| l.start as usize <= off).saturating_sub(1),
    };
    let removed = |off: usize| breaks.iter().any(|(&q, &p)| off >= q && off < p);

    // Each line's leading: its natural height as laid out plus the gap, a percent of its largest size.
    let mut line_size = vec![0.0f32; lines.len()];
    for w in cuts.windows(2) {
        let Some(run) = runs.get(run_at(w[0])) else { continue };
        let li = line_at(w[0]);
        if let Some(sz) = line_size.get_mut(li) {
            let scale = lines.get(li).map_or(1.0, |l| l.scale);
            *sz = sz.max(scaled_style(&run.style, scale).size_pt);
        }
    }
    let leading: Vec<Option<f32>> = lines
        .iter()
        .zip(&line_size)
        .map(|(l, &size)| {
            let info = fitted.lines.iter().find(|i| i.range.start == l.start as usize)?;
            Some(((info.ascent + info.descent) / k + line_gap / 100.0 * size).clamp(-MAX_PT, MAX_PT * 4.0))
        })
        .collect();

    // The solved lines span the box exactly, and wrapped text breaks a line that is a hair wider
    // than its width: every line shrinks by a margin, the smallest of MARGINS that keeps the
    // fitted line count. Each try is verified; none fitting is an error.
    let assemble = |engine: &mut crate::TextEngine, margin: f32| -> Result<TextLayer, BakeError> {
        let mut out = out.clone();
        let mut new_text = String::with_capacity(text.len());
        let mut new_runs: Vec<(usize, photocraft_doc::text::CharStyle)> = Vec::new();
        for w in cuts.windows(2) {
            let (a, z) = (w[0], w[1]);
            if a == z || removed(a) {
                continue;
            }
            let piece = text.get(a..z).ok_or(BakeError::Cut { start: a, end: z })?;
            let li = line_at(a);
            let base = runs.get(run_at(a)).map(|r| r.style.clone()).unwrap_or_default();
            let mut style = scaled_style(&base, lines.get(li).map_or(1.0, |l| l.scale) * margin);
            if let Some(lead) = leading.get(li).copied().flatten() {
                style.leading_pt = Some(lead);
            }
            if let Some(&(_, delta)) = space_extra.get(&a) {
                // Word spacing is added to the shaped advance, before the horizontal scale stretches
                // it; tracking is added after shaping and scaled with it. Divide the scale back out.
                let px = style.size_pt * k * if style.horizontal_scale > 0.0 { style.horizontal_scale } else { 1.0 };
                if px > 0.0 {
                    style.tracking += delta / px * 1000.0;
                }
            }
            new_text.push_str(piece);
            let mut len = piece.len();
            if breaks.contains_key(&z) {
                new_text.push(FORCED_LINE_BREAK);
                len += FORCED_LINE_BREAK.len_utf8();
            }
            match new_runs.last_mut() {
                Some(last) if last.1 == style => last.0 += len,
                _ => new_runs.push((len, style)),
            }
        }
        // An original offset in the new text: each break before it replaces its spaces by one char.
        let moved = |off: usize| {
            let shift: isize = breaks.iter().filter(|&(_, &p)| p <= off).map(|(&q, &p)| 1 - (p - q) as isize).sum();
            off.saturating_add_signed(shift)
        };
        let mut new_paras = Vec::new();
        let mut at = 0usize;
        for p in layer.paragraph_runs() {
            let end = at.saturating_add(p.len);
            new_paras.push(photocraft_doc::text::ParagraphRun { len: moved(end).saturating_sub(moved(at)), style: p.style });
            at = end;
        }
        out.text = new_text;
        out.runs = new_runs.into_iter().map(|(len, style)| TextRun { len, style }).collect();
        out.paragraphs = new_paras;
        out.sync_summary();
        let baked = engine.layout(&out, dpi);
        if baked.lines.len() != fitted.lines.len() {
            return Err(BakeError::LineCount { baked: baked.lines.len(), fitted: fitted.lines.len() });
        }
        if !same_glyphs(&fitted, &baked) {
            return Err(BakeError::Drift);
        }
        Ok(out)
    };
    // The cause reported: the first line-count failure, else the last failure.
    let mut cause = BakeError::Drift;
    for margin in MARGINS {
        match assemble(engine, margin) {
            Ok(b) => return Ok(b),
            Err(e) if !matches!(cause, BakeError::LineCount { .. }) => cause = e,
            Err(_) => {}
        }
    }
    Err(cause)
}

/// Exact layouts after the search: each lays out the chosen lines and corrects every scale by
/// its measured width (optical-size fonts don't scale linearly) and the block by its height.
const REFINE_PASSES: usize = 4;
/// Widths the search scans before it bisects (part of MAX_PROBES).
const SCAN_PROBES: usize = 10;
/// How far (px) the last line's descent may hang below the box and the block still fit.
const FIT_EPS: f32 = 0.5;
/// A line is done once its measured width is this close to its target (px).
const WIDTH_EPS: f32 = 0.05;

/// One word of a segment at base size: where it starts, the width of the spaces before it (since
/// the previous word, or the segment start) and its own width, in px.
struct Word {
    start: usize,
    lead: f32,
    width: f32,
    /// Largest type size among its characters (pt).
    pt: f32,
}

/// Text between mandatory breaks (paragraph breaks and forced line breaks): every segment
/// starts a line.
struct Segment {
    start: usize,
    words: Vec<Word>,
    /// Starts its paragraph (first-line indent, space before).
    para_first: bool,
    /// Ascent and descent per px of type size, from the base layout.
    ascent: f32,
    descent: f32,
    /// Size (pt) at the segment start: an empty segment's line height and gap.
    pt: f32,
    /// Paragraph metrics (px).
    avail: f32,
    first_indent: f32,
    space_before: f32,
    space_after: f32,
}

/// A candidate line: its segment, its words, and its natural width at base size (px).
#[derive(Clone)]
struct Cand {
    seg: usize,
    first: usize,
    end: usize,
    natural: f32,
}

struct Solved {
    lines: Vec<FitLine>,
    fits: bool,
    /// Height evaluations of the search (≤ MAX_PROBES).
    probes: usize,
    /// Full layouts: one to measure, then the refinement passes.
    #[cfg_attr(not(test), expect(dead_code, reason = "read by the bound tests"))]
    layouts: usize,
    /// Base layout lines visited while splitting into segments.
    #[cfg_attr(not(test), expect(dead_code, reason = "read by the bound tests"))]
    visited: usize,
}

/// Photoshop Block: lines and per-line scales so each line spans the box width and the block
/// fits the box height (spec steps 1–6). Empty text → empty Vec. Bounded: ≤ MAX_PROBES search
/// iterations and at most 1 + REFINE_PASSES layouts.
///
/// Measures every word once (one point-text layout), searches the target natural width `w`
/// (a geometric scan, then bisection) with greedy breaking over at most MAX_PROBES height
/// estimates, then lays the chosen lines
/// out exactly (at most REFINE_PASSES layouts of the whole layer) and corrects each scale by the
/// width the layout measures. A line starts only at a paragraph, a forced line break or a word
/// after a space; an empty one keeps scale 1. A box too short for one line per paragraph shrinks
/// every line by one factor. Each effective size stays within MIN_PT..=MAX_PT.
///
/// Callers must refuse vertical text, RTL text and multi-layer input before solving.
pub fn solve_box(engine: &mut crate::TextEngine, layer: &TextLayer, dpi: f32) -> Vec<FitLine> {
    solve(engine, layer, dpi).lines
}

fn solve(engine: &mut crate::TextEngine, layer: &TextLayer, dpi: f32) -> Solved {
    let none = Solved { lines: Vec::new(), fits: true, probes: 0, layouts: 0, visited: 0 };
    let TextShape::Box { y: box_y, width, height, .. } = layer.shape else { return none };
    if layer.text.is_empty() || u32::try_from(layer.text.len()).is_err() || !width.is_finite() || !height.is_finite() || !box_y.is_finite() {
        return none;
    }
    let (box_w, box_h) = (width.max(1.0), height.max(1.0));
    let k = if dpi.is_finite() && dpi > 0.0 { dpi / 72.0 } else { 1.0 };
    let line_gap = layer.fit.as_ref().map_or(0.0, |f| f.gaps().1) / 100.0;

    // The layer as the solver lays it out: non-finite sizes replaced (as `apply_scale` does), no
    // cached pixels, gaps kept.
    let mut base = layer.clone();
    base.cache = None;
    base.psd_raw = None;
    replace_non_finite(&mut base);
    let fit = base.fit.get_or_insert_with(TextFit::default);
    fit.lines.clear();

    // 1–2. Paragraphs, segments and words, measured at base size in one point-text layout.
    let mut point = base.clone();
    point.shape = TextShape::Point;
    let measured = engine.layout(&point, dpi);
    let Some((segs, visited)) = segments(&base, &measured, k, box_w) else {
        // Nothing to measure the lines with: no solved lines, and the text is not known to fit.
        return Solved { fits: false, ..none };
    };
    if segs.is_empty() {
        return none;
    }
    let mut layouts = 1;

    // 3. Search the target natural width `w`: the smallest whose greedy lines, each scaled to
    // the box width, stack no taller than the box.
    let lo_w = segs.iter().flat_map(|s| s.words.iter().enumerate().map(|(i, w)| if i == 0 { w.lead + w.width } else { w.width })).fold(0.0f32, f32::max);
    let hi_w = segs.iter().map(|s| natural(s, 0, s.words.len())).fold(lo_w, f32::max);
    let eval = |w: f32| {
        let lines = greedy(&segs, w);
        let scales: Vec<f32> = lines.iter().map(|c| width_scale(&segs, c)).collect();
        let h = stack(&segs, &lines, &scales, line_gap, k);
        (lines, scales, h)
    };
    let (mut lines, mut scales, h_hi) = eval(hi_w);
    let mut probes = 1;
    // 4. Too short: even one line per paragraph is too tall.
    let too_short = h_hi > box_h;
    if !too_short && lo_w > 0.0 && lo_w < hi_w {
        // Greedy breaking makes the height jump about with `w` (a word moving up can leave a
        // short last line that scales up a lot), so a plain bisection can miss smaller widths
        // that fit. Scan SCAN_PROBES widths from the longest word up (geometric steps), then
        // bisect between the last one too tall and the first that fits.
        let step = (hi_w / lo_w).powf(1.0 / SCAN_PROBES as f32);
        let (mut lo, mut hi) = (lo_w, hi_w);
        let mut first = true;
        for i in 0..SCAN_PROBES {
            let w = lo_w * step.powi(i as i32);
            let m = eval(w);
            probes += 1;
            if m.2 <= box_h {
                hi = w;
                (lines, scales) = (m.0, m.1);
                break;
            }
            lo = w;
            first = false;
        }
        if !first {
            while probes < MAX_PROBES && hi - lo > 0.25 {
                let mid = 0.5 * (lo + hi);
                let m = eval(mid);
                probes += 1;
                if m.2 <= box_h {
                    hi = mid;
                    (lines, scales) = (m.0, m.1);
                } else {
                    lo = mid;
                }
            }
        }
    }

    // Each line's target width: the box less its indents, times the common factor that makes
    // the block fit when the box is too short (spec step 5).
    let mut target: Vec<f32> = lines.iter().map(|c| line_target(&segs, c)).collect();
    if too_short {
        let f = shrink_factor(&segs, &lines, &scales, line_gap, k, box_h);
        let _ = shrink(&segs, &lines, &mut scales, &mut target, f);
    }

    // 5. Exact widths: lay the chosen lines out and correct each scale by the visible width
    // layout measures (kerning, horizontal scale, optical sizes), and the whole block by its measured height. The scales
    // returned are the ones last laid out.
    let starts: Vec<u32> = lines.iter().map(|c| line_start(&segs, c) as u32).collect();
    let mut fits = true;
    for pass in 0..REFINE_PASSES {
        if let Some(f) = base.fit.as_mut() {
            f.lines = starts.iter().zip(&scales).map(|(&start, &scale)| FitLine { start, scale }).collect();
        }
        let out = engine.layout(&base, dpi);
        layouts += 1;
        let bottom = out.lines.last().map_or(0.0, |l| l.baseline + l.descent) - box_y;
        // Fits: nothing hidden and the last line's descent inside the box too.
        fits = !out.hidden && bottom <= box_h + FIT_EPS;
        let tall = out.hidden || bottom > box_h;
        let mut next = scales.clone();
        let mut changed = false;
        let mut at = 0;
        for (i, c) in lines.iter().enumerate() {
            // Layout lines come in the order of the solved lines; a hidden one is missing.
            let Some(off) = out.lines.get(at..).and_then(|ls| ls.iter().position(|l| l.range.start == starts[i] as usize)) else { continue };
            at += off;
            // The visible width: horizontal scale and manual or optical kerning included.
            let advance = out.lines[at].x1 - out.lines[at].x0;
            if c.first >= c.end || advance <= 0.0 || (advance - target[i]).abs() <= WIDTH_EPS {
                continue;
            }
            let s = clamp_scale(&segs, c, scales[i] * target[i] / advance);
            changed |= s != scales[i];
            next[i] = s;
        }
        if tall && bottom > 0.0 {
            // The estimate fell short of the real height: shrink the block to fit (by more when
            // lines are hidden, as the bottom shown doesn't count them).
            let f = (box_h / bottom).min(1.0) * if out.hidden { 0.9 } else { 0.999 };
            changed |= shrink(&segs, &lines, &mut next, &mut target, f);
        }
        if !changed || pass + 1 == REFINE_PASSES {
            break;
        }
        scales = next;
    }
    let lines = starts.iter().zip(&scales).map(|(&start, &scale)| FitLine { start, scale }).collect();
    Solved { lines, fits, probes, layouts, visited }
}

/// Splits the layer into segments of words measured in `m` (the layer laid out as point text).
/// Also returns how many base layout lines it visited (tests check it stays linear).
/// None when a segment has no base layout line to measure.
fn segments(t: &TextLayer, m: &crate::TextLayout, k: f32, box_w: f32) -> Option<(Vec<Segment>, usize)> {
    use crate::layout::{FORCED_LINE_BREAK, split_paragraphs};
    let text = &t.text;
    let runs = t.char_runs();
    let mut run_ends = Vec::with_capacity(runs.len());
    let mut acc = 0usize;
    for r in &runs {
        acc = acc.saturating_add(r.len);
        run_ends.push(acc);
    }
    let pt_at = |off: usize| {
        let i = run_ends.partition_point(|&e| e <= off).min(runs.len().saturating_sub(1));
        runs.get(i).map_or(t.size_pt, |r| r.style.size_pt).clamp(MIN_PT, MAX_PT)
    };
    let paras = t.paragraph_runs();
    let mut para_ends = Vec::with_capacity(paras.len());
    let mut acc = 0usize;
    for p in &paras {
        acc = acc.saturating_add(p.len);
        para_ends.push(acc);
    }
    let para_at =
        |off: usize| paras.get(para_ends.partition_point(|&e| e <= off).min(paras.len().saturating_sub(1))).map(|p| p.style.clone()).unwrap_or_default();
    let len = |v: f32| if v.is_finite() { v * k } else { 0.0 };

    let mut clusters: Vec<&crate::ClusterInfo> = m.clusters.iter().collect();
    clusters.sort_by_key(|c| c.range.start);
    let mut lines: Vec<&crate::LineInfo> = m.lines.iter().collect();
    lines.sort_by_key(|l| l.range.start);
    let mut out = Vec::new();
    let mut visited = 0usize;
    for prange in split_paragraphs(text) {
        let content_end = prange.start + text.get(prange.clone()).unwrap_or("").trim_end_matches(['\r', '\n']).len();
        let ps = para_at(prange.start);
        let avail = (box_w - len(ps.start_indent_pt) - len(ps.end_indent_pt)).max(1.0);
        let mut bounds = vec![prange.start];
        if let Some(content) = text.get(prange.start..content_end) {
            bounds.extend(content.match_indices(FORCED_LINE_BREAK).map(|(i, b)| prange.start + i + b.len()));
        }
        for (j, &a) in bounds.iter().enumerate() {
            let z = bounds.get(j + 1).map_or(content_end, |&n| n - FORCED_LINE_BREAK.len_utf8());
            let mut words: Vec<Word> = Vec::new();
            let (mut pending, mut in_word) = (0.0f32, false);
            let first = clusters.partition_point(|c| c.range.start < a);
            for c in clusters[first..].iter().take_while(|c| c.range.start < z) {
                let piece = text.get(c.range.clone()).unwrap_or("");
                let adv = if c.advance.is_finite() { c.advance } else { 0.0 };
                if !piece.is_empty() && piece.chars().all(breaking_space) {
                    pending += adv;
                    in_word = false;
                    continue;
                }
                let pt = piece.char_indices().map(|(i, _)| pt_at(c.range.start + i)).fold(MIN_PT, f32::max);
                // A combining mark (or joiner) the shaper split from a space can't start a line:
                // it joins the word before, spaces and all.
                let glued = piece.chars().next().is_some_and(extends);
                match words.last_mut() {
                    Some(w) if in_word || glued => {
                        w.width += pending + adv;
                        w.pt = w.pt.max(pt);
                    }
                    _ => words.push(Word { start: c.range.start, lead: pending, width: adv, pt }),
                }
                pending = 0.0;
                in_word = true;
            }
            // Ascent and descent per px from the base line(s) of this segment: the font's real
            // metrics as layout measured them. Point text gives every segment a line; if one
            // has none, there is nothing true to stack it with, so the solver gives up.
            let (mut asc, mut desc, mut found) = (0.0f32, 0.0f32, false);
            let from = lines.partition_point(|l| l.range.start < a);
            for l in lines[from..].iter().take_while(|l| l.range.start <= z) {
                visited += 1;
                found = true;
                asc = asc.max(l.ascent);
                desc = desc.max(l.descent);
            }
            if !found {
                return None;
            }
            let seg_pt = words.iter().map(|w| w.pt).fold(0.0f32, f32::max);
            let pt = pt_at(a);
            let px = if seg_pt > 0.0 { seg_pt } else { pt } * k;
            let (ascent, descent) = (asc / px, desc / px);
            out.push(Segment {
                start: a,
                words,
                para_first: j == 0,
                ascent,
                descent,
                pt,
                avail,
                first_indent: len(ps.first_line_indent_pt),
                space_before: len(ps.space_before_pt),
                space_after: len(ps.space_after_pt),
            });
        }
    }
    Some((out, visited))
}

/// Whitespace a line may break after (UAX #14 classes SP and BA). No-break spaces (GL, such
/// as U+00A0 and U+202F) and word joiners stay inside their word.
fn breaking_space(c: char) -> bool {
    use icu_properties::{CodePointMapData, props::LineBreak as L};
    c.is_whitespace() && matches!(CodePointMapData::<L>::new().get(c), L::Space | L::BreakAfter)
}

/// Whether `c` extends the grapheme before it (combining marks, spacing marks, ZWJ).
fn extends(c: char) -> bool {
    use icu_properties::{CodePointMapData, props::GraphemeClusterBreak as G};
    matches!(CodePointMapData::<G>::new().get(c), G::Extend | G::SpacingMark | G::ZWJ)
}

/// Natural width (px, base size) of words `first..end` of `s` as one line. The segment's first
/// line keeps the spaces before its first word; later lines start at their word.
fn natural(s: &Segment, first: usize, end: usize) -> f32 {
    s.words.get(first..end).map_or(0.0, |ws| ws.iter().enumerate().map(|(i, w)| if i > 0 || first == 0 { w.lead + w.width } else { w.width }).sum())
}

/// Greedy lines no wider than `w` at base size (a longer single word gets its own line).
fn greedy(segs: &[Segment], w: f32) -> Vec<Cand> {
    let mut out = Vec::new();
    for (si, s) in segs.iter().enumerate() {
        if s.words.is_empty() {
            out.push(Cand { seg: si, first: 0, end: 0, natural: 0.0 });
            continue;
        }
        let mut first = 0;
        let mut width = natural(s, 0, 1);
        for (j, word) in s.words.iter().enumerate().skip(1) {
            if width + word.lead + word.width <= w {
                width += word.lead + word.width;
            } else {
                out.push(Cand { seg: si, first, end: j, natural: width });
                first = j;
                width = word.width;
            }
        }
        out.push(Cand { seg: si, first, end: s.words.len(), natural: width });
    }
    out
}

fn line_start(segs: &[Segment], c: &Cand) -> usize {
    let s = &segs[c.seg];
    if c.first == 0 { s.start } else { s.words.get(c.first).map_or(s.start, |w| w.start) }
}

/// Width the line must span: the box less the paragraph's indents, and on its first line the
/// first-line indent (as layout breaks it).
fn line_target(segs: &[Segment], c: &Cand) -> f32 {
    let s = &segs[c.seg];
    let first_line = s.para_first && c.first == 0;
    (s.avail - if first_line { s.first_indent } else { 0.0 }).max(1.0)
}

/// Largest type size on the line (pt).
fn line_pt(segs: &[Segment], c: &Cand) -> f32 {
    let s = &segs[c.seg];
    s.words.get(c.first..c.end).map_or(s.pt, |ws| ws.iter().map(|w| w.pt).fold(MIN_PT, f32::max))
}

/// `scale` limited so the line's largest size stays within MIN_PT..=MAX_PT; empty lines keep 1.
fn clamp_scale(segs: &[Segment], c: &Cand, scale: f32) -> f32 {
    if c.first >= c.end {
        return 1.0;
    }
    let pt = line_pt(segs, c).max(MIN_PT);
    let s = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    s.clamp(MIN_PT / pt, MAX_PT / pt)
}

/// The scale that makes the line span its target width (clamped).
fn width_scale(segs: &[Segment], c: &Cand) -> f32 {
    if c.first >= c.end || c.natural <= 0.0 {
        return clamp_scale(segs, c, 1.0);
    }
    clamp_scale(segs, c, line_target(segs, c) / c.natural)
}

/// Height (px) from the box top to the last line's descent, stacked as layout stacks solved
/// lines: the first line's ascent, then each line's ascent + descent + line gap (percent of its
/// largest size), plus paragraph spacing, then the last descent.
fn stack(segs: &[Segment], lines: &[Cand], scales: &[f32], line_gap: f32, k: f32) -> f32 {
    let (fixed, scaled) = stack_parts(segs, lines, scales, line_gap, k);
    fixed + scaled
}

/// [`stack`] split into the part that doesn't follow the line scales (paragraph spacing and
/// empty lines) and the part that does.
fn stack_parts(segs: &[Segment], lines: &[Cand], scales: &[f32], line_gap: f32, k: f32) -> (f32, f32) {
    let (mut fixed, mut scaled) = (0.0f32, 0.0f32);
    let n = lines.len();
    for (i, (c, &s)) in lines.iter().zip(scales).enumerate() {
        let seg = &segs[c.seg];
        let empty = c.first >= c.end;
        let px = line_pt(segs, c) * k * if empty { 1.0 } else { s };
        let (asc, desc) = (seg.ascent * px, seg.descent * px);
        let mut h = if i == 0 { asc } else { asc + desc + line_gap * px };
        if i + 1 == n {
            h += desc;
        }
        if i > 0 && seg.para_first && c.first == 0 {
            let prev = &segs[lines[i - 1].seg];
            fixed += prev.space_after + seg.space_before;
        }
        if empty {
            fixed += h;
        } else {
            scaled += h;
        }
    }
    (fixed, scaled)
}

/// Multiplies the scale and target of every non-empty line by `f`. Whether any scale changed
/// (empty lines and clamped ones can't shrink).
fn shrink(segs: &[Segment], lines: &[Cand], scales: &mut [f32], target: &mut [f32], f: f32) -> bool {
    let mut changed = false;
    for ((s, t), c) in scales.iter_mut().zip(target.iter_mut()).zip(lines) {
        if c.first < c.end {
            let next = clamp_scale(segs, c, *s * f);
            changed |= next != *s;
            *s = next;
            *t *= f;
        }
    }
    changed
}

/// The common factor that makes the stacked block `box_h` tall (spec step 5).
fn shrink_factor(segs: &[Segment], lines: &[Cand], scales: &[f32], line_gap: f32, k: f32, box_h: f32) -> f32 {
    let (fixed, scaled) = stack_parts(segs, lines, scales, line_gap, k);
    if scaled <= 0.0 {
        return 1.0;
    }
    ((box_h - fixed) / scaled).clamp(1e-6, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TextEngine;
    use crate::tests::{area_ellipse_layer, box_layer};
    use photocraft_doc::text::TextFit;
    const FORCED_BREAK: &str = "\u{3}";

    /// The uniform fit (what Area text uses), applied to any layer: Box text no longer takes it
    /// through `refit`, but `fit_scale`/`apply_scale` still work on it.
    fn uniform(e: &mut TextEngine, t: &mut TextLayer) -> FitResult {
        let r = fit_scale(e, t, 72.0);
        apply_scale(t, r.scale);
        r
    }

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
        let r = uniform(&mut e, &mut t);
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
        uniform(&mut e, &mut narrow);
        uniform(&mut e, &mut wide);
        assert!(wide.size_pt > narrow.size_pt);
        assert!(e.layout(&wide, 72.0).lines.len() < e.layout(&narrow, 72.0).lines.len());
    }

    #[test]
    fn long_text_bottoms_out_at_minimum() {
        let mut e = TextEngine::new();
        let mut t = fit_box(&"word ".repeat(5000), 12.0, 2.0, 2.0);
        let r = uniform(&mut e, &mut t);
        assert!(!r.fits && r.probes <= MAX_PROBES);
        assert!((t.size_pt - MIN_PT).abs() < 1e-4);
    }

    #[test]
    fn mixed_sizes_keep_their_ratio() {
        let mut e = TextEngine::new();
        let mut t = fit_box("big small", 12.0, 400.0, 200.0);
        set_run_sizes(&mut t, &[(3, 30.0), (6, 10.0)]); // "big" 30pt, " small" 10pt
        uniform(&mut e, &mut t);
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
    fn area_fit_keeps_run_ratios_at_the_top_end() {
        // A shape far bigger than the type can grow: the largest run reaches MAX_PT first, and
        // the smaller one keeps its 1:10 ratio instead of catching up at the clamp.
        let mut e = TextEngine::new();
        let mut t = area_ellipse_layer("big small", 12.0, 60_000.0, 60_000.0);
        set_run_sizes(&mut t, &[(3, 10.0), (6, 100.0)]); // "big" 10pt, " small" 100pt
        t.fit = Some(TextFit::default());
        refit(&mut e, &mut t, 72.0);
        let s: Vec<f32> = t.runs.iter().map(|r| r.style.size_pt).collect();
        assert!((s[1] / s[0] - 10.0).abs() < 1e-3, "{s:?}");
        assert!((s[1] - MAX_PT).abs() < 1.0, "{s:?}");
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
        let r = uniform(&mut e, &mut t);
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
        let r = uniform(&mut e, &mut u);
        assert!(r.scale.is_finite() && u.size_pt.is_finite() && u.runs.iter().all(|r| r.style.size_pt.is_finite()));
        // The Box solver never writes sizes, and its scales stay finite on non-finite sizes.
        let mut v = fit_box("ab cd", f32::NAN, 100.0, 100.0);
        set_run_sizes(&mut v, &[(2, f32::NAN), (3, f32::INFINITY)]);
        let lines = solve_box(&mut e, &v, 72.0);
        assert!(!lines.is_empty() && lines.iter().all(|l| l.scale.is_finite() && l.scale > 0.0), "{lines:?}");
    }

    const FOX: &str = "The quick brown fox jumps over the lazy dog";

    /// `t` with its solved lines written, and its layout.
    fn solved(e: &mut TextEngine, mut t: TextLayer) -> (TextLayer, crate::TextLayout) {
        let lines = solve_box(e, &t, 72.0);
        if let Some(f) = t.fit.as_mut() {
            f.lines = lines;
        }
        let l = e.layout(&t, 72.0);
        (t, l)
    }

    fn scales(t: &TextLayer) -> Vec<f32> {
        t.fit.as_ref().map(|f| f.lines.iter().map(|l| l.scale).collect()).unwrap_or_default()
    }

    fn widths(l: &crate::TextLayout) -> Vec<f32> {
        l.lines.iter().map(|li| li.x1 - li.x0).collect()
    }

    fn bottom(l: &crate::TextLayout) -> f32 {
        l.lines.last().map_or(0.0, |li| li.baseline + li.descent)
    }

    fn assert_spans(l: &crate::TextLayout, w: f32) {
        for (i, got) in widths(l).into_iter().enumerate() {
            assert!((got - w).abs() <= 1.5, "line {i} is {got} px wide, box {w}: {:?}", widths(l));
        }
    }

    #[test]
    fn every_line_spans_the_box_width() {
        let mut e = TextEngine::new();
        let (t, l) = solved(&mut e, fit_box(FOX, 12.0, 400.0, 300.0));
        let s = scales(&t);
        assert!(s.len() >= 2 && l.lines.len() == s.len(), "{s:?}");
        assert_spans(&l, 400.0);
        let (lo, hi) = s.iter().fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)));
        assert!(hi - lo > 1e-3, "lines of different lengths get different sizes: {s:?}");
    }

    #[test]
    fn short_line_is_bigger_and_both_span_width() {
        let mut e = TextEngine::new();
        let (t, l) = solved(&mut e, fit_box("I AM A\nEXPANDABLE", 12.0, 400.0, 300.0));
        let s = scales(&t);
        assert_eq!(s.len(), 2, "{s:?}");
        assert!(s[0] > s[1], "{s:?}");
        assert_spans(&l, 400.0);
    }

    #[test]
    fn narrower_box_gives_more_lines() {
        let mut e = TextEngine::new();
        let (wide, _) = solved(&mut e, fit_box(FOX, 12.0, 600.0, 300.0));
        let (narrow, _) = solved(&mut e, fit_box(FOX, 12.0, 200.0, 300.0));
        assert!(scales(&narrow).len() > scales(&wide).len(), "{:?} vs {:?}", scales(&narrow), scales(&wide));
    }

    #[test]
    fn block_fits_box_height() {
        let mut e = TextEngine::new();
        for (text, w, h, line_gap) in
            [(FOX, 400.0, 300.0, 0.0), (FOX, 300.0, 500.0, 50.0), ("I AM A\nEXPANDABLE", 400.0, 120.0, -50.0), (FOX, 250.0, 90.0, 200.0)]
        {
            let mut t = fit_box(text, 12.0, w, h);
            t.fit = Some(TextFit { line_gap, ..Default::default() });
            let (_, l) = solved(&mut e, t);
            assert!(bottom(&l) <= h + 0.5, "{text:?} {w}x{h} gap {line_gap}: bottom {}", bottom(&l));
            assert!(!l.overflowed && !l.hidden, "{text:?} {w}x{h} gap {line_gap}");
        }
    }

    #[test]
    fn too_short_box_shrinks_all_lines() {
        let mut e = TextEngine::new();
        let (t, l) = solved(&mut e, fit_box("ONE\nTWO WORDS\nTHREE", 12.0, 400.0, 20.0));
        assert_eq!(scales(&t).len(), 3, "one line per paragraph: {:?}", scales(&t));
        assert!(widths(&l).iter().all(|&w| w < 399.0), "{:?}", widths(&l));
        assert!(bottom(&l) <= 20.5 && !l.hidden, "bottom {}", bottom(&l));
    }

    #[test]
    fn one_long_word_is_bounded() {
        let mut e = TextEngine::new();
        let (t, l) = solved(&mut e, fit_box(&"a".repeat(500), 12.0, 50.0, 50.0));
        let s = scales(&t);
        assert!(!s.is_empty() && s.iter().all(|v| v.is_finite() && *v > 0.0), "{s:?}");
        assert!(!l.hidden);
    }

    #[test]
    fn word_gap_minus_100_removes_spaces() {
        let mut e = TextEngine::new();
        let natural = |e: &mut TextEngine, text: &str| {
            let mut t = fit_box(text, 12.0, 400.0, 400.0);
            t.fit = Some(TextFit { word_gap: -100.0, ..Default::default() });
            let s = scales(&solved(e, t).0);
            assert_eq!(s.len(), 1, "{text:?}: {s:?}");
            400.0 / s[0]
        };
        let (spaced, joined) = (natural(&mut e, "aa bb"), natural(&mut e, "aabb"));
        assert!((spaced - joined).abs() <= 1.0, "aa bb {spaced} vs aabb {joined}");
    }

    #[test]
    fn refit_box_keeps_base_sizes() {
        let mut e = TextEngine::new();
        let mut t = fit_box(FOX, 12.0, 400.0, 300.0);
        set_run_sizes(&mut t, &[(10, 20.0), (FOX.len() - 10, 12.0)]);
        let runs = t.runs.clone();
        let r = refit(&mut e, &mut t, 72.0);
        assert_eq!(t.size_pt, 12.0);
        assert_eq!(t.runs, runs);
        assert!(r.fits && r.scale == 1.0 && r.probes <= MAX_PROBES, "{r:?}");
        assert!(!t.fit.as_ref().is_none_or(|f| f.lines.is_empty()));
        // Mixed sizes keep their ratio on a line: the solved layout spans the width all the same.
        assert_spans(&e.layout(&t, 72.0), 400.0);
    }

    #[test]
    fn empty_paragraphs_and_breaks() {
        let mut e = TextEngine::new();
        let text = "ONE TWO\n\nTHREE\u{3}FOUR FIVE\n";
        let (t, l) = solved(&mut e, fit_box(text, 12.0, 300.0, 600.0));
        let lines = &t.fit.as_ref().unwrap().lines;
        let starts: Vec<u32> = lines.iter().map(|l| l.start).collect();
        // Every paragraph and forced line break starts a line; empty paragraphs keep scale 1.
        for must in [0, 8, 9, 15, 25] {
            assert!(starts.contains(&must), "{starts:?}");
        }
        for (i, line) in lines.iter().enumerate() {
            let piece = &text[line.start as usize..lines.get(i + 1).map_or(text.len(), |n| n.start as usize)];
            if piece.trim_end_matches(['\n', '\u{3}']).is_empty() {
                assert_eq!(line.scale, 1.0, "empty line {i} {starts:?}");
            }
        }
        assert!(!l.hidden && bottom(&l) <= 600.5);
    }

    #[test]
    fn lines_start_at_word_boundaries_not_inside_clusters() {
        let mut e = TextEngine::new();
        // Combining acute accents (U+0301) after base letters and after a space.
        // No-break spaces (U+00A0, U+202F) glue their neighbours: never a line start after one.
        let text = "cafe\u{301} re\u{301}sume\u{301} \u{301}x na\u{ef}ve words 10\u{a0}km here 5\u{202f}kg and 20\u{a0}km there 7\u{202f}kg";
        let (t, _) = solved(&mut e, fit_box(text, 12.0, 120.0, 400.0));
        let lines = &t.fit.as_ref().unwrap().lines;
        assert!(lines.len() >= 2, "{lines:?}");
        for l in lines.iter().skip(1) {
            let at = l.start as usize;
            assert!(text.is_char_boundary(at));
            let next = text[at..].chars().next().unwrap();
            assert!(next != '\u{301}' && !next.is_whitespace(), "line at {at} starts with {next:?}");
            assert!(text[..at].ends_with(' '), "line at {at} doesn't follow a space");
        }
    }

    #[test]
    fn hostile_input_is_bounded() {
        let mut e = TextEngine::new();
        for t in [
            fit_box(&"word ".repeat(3000), 12.0, 400.0, 300.0),
            fit_box(FOX, 12.0, 0.0, 0.0),
            fit_box(FOX, 12.0, f32::NAN, f32::INFINITY),
            fit_box(FOX, 1296.0, 1e6, 1e6),
            fit_box(FOX, 0.1, 1.0, 1e6),
            fit_box("   ", 12.0, 400.0, 300.0),
        ] {
            let lines = solve_box(&mut e, &t, 72.0);
            assert!(lines.iter().all(|l| l.scale.is_finite() && l.scale > 0.0));
            let mut f = t.fit.clone().unwrap();
            f.lines = lines.clone();
            assert_eq!(f.clone().sanitized(&t.text).lines, lines, "valid lines");
        }
        assert!(solve_box(&mut e, &fit_box("", 12.0, 400.0, 300.0), 72.0).is_empty());
    }

    #[test]
    fn search_and_layouts_are_bounded() {
        let mut e = TextEngine::new();
        let long = "lorem ipsum dolor sit amet ".repeat(40);
        let cases: [(&str, f32, f32, f32); 8] = [
            (FOX, 400.0, 300.0, 12.0),
            (FOX, 600.0, 300.0, 12.0),
            (FOX, 200.0, 300.0, 12.0),
            (
                "I AM A
EXPANDABLE",
                400.0,
                300.0,
                12.0,
            ),
            (
                "ONE
TWO WORDS
THREE",
                400.0,
                20.0,
                12.0,
            ),
            (&long, 500.0, 700.0, 12.0),
            (&long, 300.0, 2000.0, 72.0),
            ("Hi", 200.0, 80.0, 12.0),
        ];
        for (text, w, h, size) in cases {
            let t = fit_box(text, size, w, h);
            let r = solve(&mut e, &t, 72.0);
            assert!(r.probes <= MAX_PROBES && r.layouts <= 1 + REFINE_PASSES, "{text:?}: {} probes, {} layouts", r.probes, r.layouts);
            let mut t2 = t.clone();
            t2.fit.as_mut().unwrap().lines = r.lines.clone();
            let l = e.layout(&t2, 72.0);
            let err = l.lines.iter().map(|li| (li.x1 - li.x0 - w).abs()).fold(0.0f32, f32::max);
            eprintln!(
                "{:.24?} {w}x{h} {size}pt: {} lines, {} probes, {} layouts, max width error {err:.3} px, bottom {:.2}, hidden {}",
                text,
                r.lines.len(),
                r.probes,
                r.layouts,
                bottom(&l),
                l.hidden
            );
        }
    }

    #[test]
    fn indents_shorten_each_lines_target() {
        let mut e = TextEngine::new();
        let mut t = fit_box(FOX, 12.0, 400.0, 300.0);
        with_para(
            &mut t,
            photocraft_doc::text::ParagraphStyle { start_indent_pt: 20.0, end_indent_pt: 10.0, first_line_indent_pt: 30.0, ..Default::default() },
        );
        let (t, l) = solved(&mut e, t);
        assert!(scales(&t).len() >= 2, "{:?}", scales(&t));
        let w = widths(&l);
        assert!((w[0] - 340.0).abs() <= 1.5, "first line takes the first-line indent: {w:?}");
        assert!(w[1..].iter().all(|v| (v - 370.0).abs() <= 1.5), "{w:?}");
        assert!(!l.overflowed);
    }

    #[test]
    fn horizontal_scale_and_kerning_lines_span_the_visible_width() {
        let mut e = TextEngine::new();
        for (hs, kern) in [(1.5, 40.0), (0.8, -30.0)] {
            let mut t = fit_box(FOX, 12.0, 400.0, 300.0);
            set_run_sizes(&mut t, &[(FOX.len(), 12.0)]);
            t.runs[0].style.horizontal_scale = hs;
            t.runs[0].style.kern = kern;
            let (t, l) = solved(&mut e, t);
            assert!(scales(&t).len() >= 2, "{:?}", scales(&t));
            assert_spans(&l, 400.0);
            assert!(!l.overflowed, "hs {hs} kern {kern}: solved lines aren't flagged too wide");
        }
    }

    #[test]
    fn many_short_lines_stay_linear() {
        let mut e = TextEngine::new();
        let text = "ab cd
"
        .repeat(5000);
        let t = fit_box(&text, 12.0, 400.0, 300.0);
        let r = solve(&mut e, &t, 72.0);
        assert_eq!(r.lines.len(), 5001, "one line per paragraph (the box is too short)");
        assert!(r.probes <= MAX_PROBES && r.layouts <= 1 + REFINE_PASSES, "{} probes, {} layouts", r.probes, r.layouts);
        assert!(r.visited <= 2 * r.lines.len(), "{} base lines visited for {} segments", r.visited, r.lines.len());
    }

    #[test]
    fn refit_box_replaces_only_non_finite_sizes() {
        let mut e = TextEngine::new();
        let mut t = fit_box("ab cd ef", f32::NAN, 300.0, 200.0);
        set_run_sizes(&mut t, &[(2, f32::NAN), (3, 30.0), (3, f32::NEG_INFINITY)]);
        t.runs[1].style.leading_pt = Some(f32::NAN);
        t.runs[1].style.baseline_shift_pt = f32::INFINITY;
        refit(&mut e, &mut t, 72.0);
        assert!(t.size_pt.is_finite() && t.runs.iter().all(|r| r.style.size_pt.is_finite()), "{:?}", t.runs);
        assert_eq!(t.runs[1].style.size_pt, 30.0, "a valid base size is untouched");
        assert!(t.runs[1].style.leading_pt.is_none_or(f32::is_finite) && t.runs[1].style.baseline_shift_pt.is_finite());
        // Valid sizes, even outside the type range, are the user's base: left alone.
        let mut u = fit_box("ab cd", 12.0, 300.0, 200.0);
        set_run_sizes(&mut u, &[(2, 2000.0), (3, 0.05)]);
        let runs = u.runs.clone();
        refit(&mut e, &mut u, 72.0);
        assert_eq!(u.runs, runs);
    }

    #[test]
    fn long_text_in_a_tiny_box_bottoms_out_at_minimum() {
        let mut e = TextEngine::new();
        let mut t = fit_box(&"word ".repeat(5000), 12.0, 2.0, 2.0);
        let r = refit(&mut e, &mut t, 72.0);
        let s = scales(&t);
        assert!(!s.is_empty() && s.iter().all(|v| v.is_finite() && *v > 0.0) && r.probes <= MAX_PROBES);
        let smallest = s.iter().map(|v| v * t.size_pt).fold(f32::MAX, f32::min);
        assert!((smallest - MIN_PT).abs() < 1e-4, "smallest effective size {smallest}");
    }

    #[test]
    fn box_indents_wider_than_the_box_stay_finite_and_fit() {
        let mut e = TextEngine::new();
        let mut t = fit_box("one two three four", 12.0, 100.0, 200.0);
        with_para(&mut t, photocraft_doc::text::ParagraphStyle { start_indent_pt: 60.0, end_indent_pt: 60.0, ..Default::default() });
        let (t, l) = solved(&mut e, t);
        let s = scales(&t);
        assert!(!s.is_empty() && s.iter().all(|v| v.is_finite() && *v > 0.0), "{s:?}");
        assert!(!l.hidden && bottom(&l) <= 200.5, "bottom {}", bottom(&l));
    }

    #[test]
    fn fits_means_the_whole_block_is_inside_the_box() {
        let mut e = TextEngine::new();
        // Two empty paragraphs (scale 1, nothing to shrink): the second baseline is inside the
        // box but its descent hangs below, so nothing is hidden and the block still doesn't fit.
        let mut big = fit_box(
            "
", 12.0, 400.0, 400.0,
        );
        refit(&mut e, &mut big, 72.0);
        let full = bottom(&e.layout(&big, 72.0));
        let mut t = fit_box(
            "
",
            12.0,
            400.0,
            full - 1.5,
        );
        let r = refit(&mut e, &mut t, 72.0);
        let l = e.layout(&t, 72.0);
        assert!(!l.hidden && bottom(&l) > full - 1.0, "{} vs box {}", bottom(&l), full - 1.5);
        assert!(!r.fits, "{r:?}");
        assert!(refit(&mut e, &mut big, 72.0).fits);
    }

    /// Every glyph of `b` sits within 0.5 px of the same glyph of `a`, in order. A fitted layout
    /// shapes each solved line on its own and so has an invisible direction mark glyph per line
    /// that the baked paragraph does not: those are skipped.
    fn assert_same_look(a: &crate::TextLayout, b: &crate::TextLayout) {
        assert!(!b.glyphs.is_empty());
        let mut from = 0;
        for (i, q) in b.glyphs.iter().enumerate() {
            let found = a.glyphs.get(from..).and_then(|rest| rest.iter().position(|p| p.id == q.id && (p.x - q.x).abs() <= 0.5 && (p.y - q.y).abs() <= 0.5));
            let Some(off) = found else { panic!("glyph {i} at ({}, {}) has no match in the fitted layout", q.x, q.y) };
            from += off + 1;
        }
        // A direction mark per fitted line, and the space each baked break replaced.
        assert!(a.glyphs.len() - b.glyphs.len() <= 2 * a.lines.len(), "{} vs {} glyphs", a.glyphs.len(), b.glyphs.len());
    }

    fn baked_matches(t: TextLayer) -> TextLayer {
        let mut e = TextEngine::new();
        let (t, fitted) = solved(&mut e, t);
        let baked = bake(&mut e, &t, 72.0).unwrap();
        assert!(baked.fit.is_none());
        assert_same_look(&fitted, &e.layout(&baked, 72.0));
        baked
    }

    fn sizes(t: &TextLayer) -> Vec<f32> {
        let mut v: Vec<f32> = t.char_runs().iter().map(|r| r.style.size_pt).collect();
        v.dedup();
        v
    }

    #[test]
    fn bake_keeps_the_look_and_gives_each_line_its_size() {
        let baked = baked_matches(fit_box(FOX, 12.0, 400.0, 300.0));
        assert!(sizes(&baked).len() >= 2, "{:?}", sizes(&baked));
        assert!(baked.text.contains(FORCED_BREAK), "{:?}", baked.text);
        assert_eq!(baked.text.replace(FORCED_BREAK, " "), FOX, "each break replaces a space");
        let total: usize = baked.char_runs().iter().map(|r| r.len).sum();
        assert_eq!(total, baked.text.len());
        let para: usize = baked.paragraph_runs().iter().map(|r| r.len).sum();
        assert_eq!(para, baked.text.len());
    }

    #[test]
    fn bake_keeps_the_look_with_gaps() {
        for (word, line) in [(50.0, 0.0), (-100.0, 25.0), (0.0, -30.0), (120.0, 80.0)] {
            let mut t = fit_box(FOX, 12.0, 400.0, 400.0);
            if let Some(f) = t.fit.as_mut() {
                f.word_gap = word;
                f.line_gap = line;
            }
            baked_matches(t);
        }
    }

    #[test]
    fn bake_does_not_double_existing_breaks() {
        let text = "ONE TWO\n\nTHREE\u{3}FOUR FIVE\n";
        let baked = baked_matches(fit_box(text, 12.0, 300.0, 600.0));
        assert!(!baked.text.contains("\u{3}\u{3}") && !baked.text.contains("\n\u{3}") && !baked.text.contains("\u{3}\n"), "{:?}", baked.text);
        assert!(baked.text.contains("THREE\u{3}FOUR"));
    }

    #[test]
    fn bake_keeps_mixed_runs_and_paragraph_styles() {
        let mut t = fit_box(FOX, 12.0, 300.0, 400.0);
        set_run_sizes(&mut t, &[(10, 20.0), (FOX.len() - 10, 12.0)]);
        let style = photocraft_doc::text::ParagraphStyle { first_line_indent_pt: 20.0, space_after_pt: 6.0, ..Default::default() };
        t.paragraphs = vec![photocraft_doc::text::ParagraphRun { len: FOX.len(), style }];
        let baked = baked_matches(t);
        assert_eq!(baked.paragraph_runs().len(), 1);
    }

    #[test]
    fn bake_of_unfitted_or_area_text_only_drops_the_fit() {
        let mut e = TextEngine::new();
        let mut t = box_layer(FOX, 12.0, (0.0, 0.0, 300.0, 100.0));
        assert_eq!(bake(&mut e, &t, 72.0).unwrap(), t);
        t.fit = Some(TextFit::default());
        let b = bake(&mut e, &t, 72.0).unwrap();
        assert!(b.fit.is_none() && b.text == t.text, "no solved lines: nothing to bake");
        let mut a = area_ellipse_layer("Hello world", 12.0, 200.0, 100.0);
        a.fit = Some(TextFit::default());
        let b = bake(&mut e, &a, 72.0).unwrap();
        assert!(b.fit.is_none() && b.text == a.text);
    }

    #[test]
    fn bake_sizes_are_clamped_and_finite() {
        let mut e = TextEngine::new();
        let mut t = fit_box("hi", 1000.0, 5000.0, 5000.0);
        if let Some(f) = t.fit.as_mut() {
            f.lines = vec![FitLine { start: 0, scale: 50.0 }];
        }
        let b = bake(&mut e, &t, 72.0).unwrap();
        assert!(b.char_runs().iter().all(|r| r.style.size_pt <= MAX_PT && r.style.size_pt >= MIN_PT));
    }

    #[test]
    fn bake_replaces_line_ending_spaces_with_the_break() {
        let baked = baked_matches(fit_box(FOX, 12.0, 400.0, 300.0));
        assert!(!baked.text.contains(" \u{3}"), "{:?}", baked.text);
        assert!(
            baked.char_runs().iter().all(|r| r.style.tracking == 0.0),
            "plain runs: {:?}",
            baked.char_runs().iter().map(|r| r.style.tracking).collect::<Vec<_>>()
        );
        // Typing at a former line end after a release: type_fit_cmds' tests type there.
    }

    #[test]
    fn bake_keeps_the_look_with_horizontal_scale_and_word_gap() {
        let make = |hs: f32| {
            let mut t = fit_box(FOX, 12.0, 400.0, 400.0);
            t.runs = vec![TextRun { len: FOX.len(), style: photocraft_doc::text::CharStyle { horizontal_scale: hs, ..t.char_runs()[0].style.clone() } }];
            if let Some(f) = t.fit.as_mut() {
                f.word_gap = 60.0;
            }
            t
        };
        // Wider type: the word gap tracking is not scaled twice.
        baked_matches(make(1.3));
        // Condensed type: box wrapping measures the unscaled width, which is wider than the box,
        // so baked lines would wrap. It is refused before baking, naming the cause.
        let mut e = TextEngine::new();
        let (t, _) = solved(&mut e, make(0.8));
        let m = bake(&mut e, &t, 72.0).unwrap_err().to_string();
        assert!(m.contains("condensed type (Horizontal Scale below 100%)"), "{m}");
    }

    #[test]
    fn same_glyphs_allows_only_a_few_unmatched_glyphs_per_line() {
        let mut e = TextEngine::new();
        let full = e.layout(&box_layer("Hello world and more words", 12.0, (0.0, 0.0, 400.0, 100.0)), 72.0);
        assert_eq!(full.lines.len(), 1);
        // Dropping a whole word leaves many fitted glyphs unmatched: not the same look.
        let short = e.layout(&box_layer("Hello world", 12.0, (0.0, 0.0, 400.0, 100.0)), 72.0);
        assert!(!same_glyphs(&full, &short));
        // Dropping one trailing space (as a baked break does) still is.
        let spaced = e.layout(&box_layer("Hello world ", 12.0, (0.0, 0.0, 400.0, 100.0)), 72.0);
        assert!(same_glyphs(&spaced, &short));
    }

    #[test]
    fn bake_reports_the_line_count_when_every_margin_fails() {
        let mut e = TextEngine::new();
        // A line 0.05 % wider than a wide box: the first margin leaves it wrapping, the second
        // keeps the line count but moves the far glyphs more than half a pixel.
        let (mut t, _) = solved(&mut e, fit_box(FOX, 12.0, 1500.0, 300.0));
        if let Some(f) = t.fit.as_mut() {
            f.lines.iter_mut().for_each(|l| l.scale *= 1.0005);
        }
        let m = bake(&mut e, &t, 72.0).unwrap_err().to_string();
        assert!(m.contains("lays out in"), "{m}");
    }

    #[test]
    fn bake_refuses_a_look_it_cannot_keep() {
        let mut e = TextEngine::new();
        // One solved line for text far wider than the box: the fitted layout never wraps, baked text would.
        let mut t = fit_box(FOX, 12.0, 100.0, 300.0);
        if let Some(f) = t.fit.as_mut() {
            f.lines = vec![FitLine { start: 0, scale: 1.0 }];
        }
        let err = bake(&mut e, &t, 72.0).unwrap_err().to_string();
        assert!(err.contains("lays out in"), "{err}");
    }

    #[test]
    fn bake_goes_past_the_first_margin_and_keeps_half_a_pixel() {
        let mut e = TextEngine::new();
        let (mut t, _) = solved(&mut e, fit_box(FOX, 12.0, 400.0, 400.0));
        // Lines 0.05 % wider than the box: the first margin (0.001 %) leaves them wrapping.
        if let Some(f) = t.fit.as_mut() {
            f.lines.iter_mut().for_each(|l| l.scale *= 1.0005);
        }
        let fitted = e.layout(&t, 72.0);
        let baked = bake(&mut e, &t, 72.0).expect("a later margin keeps the lines");
        let bl = e.layout(&baked, 72.0);
        assert_same_look(&fitted, &bl);
    }
}
