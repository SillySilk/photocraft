//! Image › Adjustments › Replace Color… as Photoshop lays it out: Localized Color Clusters, the
//! three eyedroppers (sample, add, subtract) with the Color swatch, Fuzziness, Range, Hue,
//! Saturation and Lightness, and Preview. The eyedroppers are armed as soon as the dialog opens:
//! the image itself is the eyedropper (Shift adds to the sample, Alt subtracts), reusing Select ›
//! Color Range's picking (`color_range_ui::pick`) so both dialogs build identical samples.
//!
//! Everything lives in the dialog fields, so `ui.dialog.set` drives it. `__tool` is the selected
//! eyedropper; `points`, `subtractPoints` and `order` are the engine's own params.

use egui::Color32;
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::theme::Tokens;
use crate::widgets;

pub const COMMAND: &str = "image.adjustments.replaceColor";

/// Whether `f` is the Replace Color dialog.
pub(crate) fn owns(f: &Map<String, Value>) -> bool {
    f.get("__filter").and_then(Value::as_bool) == Some(true) && f.get("__command").and_then(Value::as_str) == Some(COMMAND)
}

/// Open (and Alt+Reset) state: the plain eyedropper armed, no samples yet.
pub(crate) fn defaults(f: &mut Map<String, Value>) {
    f.insert("__tool".into(), json!("sample"));
    f.insert("points".into(), json!([]));
    f.insert("subtractPoints".into(), json!([]));
    f.insert("order".into(), json!([]));
}

fn non_empty(f: &Map<String, Value>, k: &str) -> bool {
    f.get(k).and_then(Value::as_array).is_some_and(|a| !a.is_empty())
}

/// The `image.adjustments.replaceColor` params for the dialog's fields, for the live preview and
/// OK alike (both go through `filter_dialog::params_of`). The engine rejects Localized Color
/// Clusters without a sampled position, so `localized` and `range` are only sent once there is
/// one; empty sample lists are left out.
pub(crate) fn params(f: &Map<String, Value>) -> Value {
    let mut p: Map<String, Value> = f.iter().filter(|(k, _)| !k.starts_with("__")).map(|(k, v)| (k.clone(), v.clone())).collect();
    for k in ["points", "subtractPoints", "order"] {
        if !non_empty(f, k) {
            p.remove(k);
        }
    }
    if !non_empty(f, "points") {
        p.remove("localized");
        p.remove("range");
    }
    Value::Object(p)
}

/// The live preview runs on a copy reduced by `k`: bring the eyedropper samples (document pixels)
/// to the pixel of the copy that covers them, as Color Range's preview does.
pub(crate) fn scale_samples_for_proxy(p: &mut Value, s: &photocraft_engine::Session, k: u32) {
    let Some(area) = s.active().map(|st| st.doc.bounds()) else { return };
    let (w, h) = (f64::from(area.width().max(1)), f64::from(area.height().max(1)));
    let Some(o) = p.as_object_mut() else { return };
    for key in ["points", "subtractPoints"] {
        let Some(Value::Array(a)) = o.get_mut(key) else { continue };
        for v in a.iter_mut() {
            if let Some([x, y]) = v.as_array().map(Vec::as_slice).and_then(|s| match s {
                [x, y] => Some([x.as_f64()?, y.as_f64()?]),
                _ => None,
            }) {
                *v = json!([(x / f64::from(k)).floor().clamp(0.0, w - 1.0), (y / f64::from(k)).floor().clamp(0.0, h - 1.0)]);
            }
        }
    }
}

/// Whether the top dialog is Replace Color: the canvas is its eyedropper from the moment it opens.
pub(crate) fn sampling_top(app: &PhotocraftApp) -> bool {
    app.ui.dialogs.last().is_some_and(|d| owns(&d.fields))
}

/// An eyedropper click at document point `at` (Shift adds, Alt subtracts). A plain click samples:
/// it replaces the samples and sets From `color` to the clicked pixel's merged colour. Returns
/// whether the click landed on the image.
pub(crate) fn pick_top(app: &mut PhotocraftApp, at: [f64; 2], mods: egui::Modifiers) -> bool {
    let Some(mut f) = app.ui.dialogs.last().map(|d| &d.fields).filter(|f| owns(f)).cloned() else { return false };
    let Some(doc) = app.session.active().map(|st| &st.doc) else { return false };
    if !at[0].is_finite() || !at[1].is_finite() || at[0] < 0.0 || at[1] < 0.0 || at[0] >= f64::from(doc.size.width) || at[1] >= f64::from(doc.size.height) {
        return false;
    }
    let tool = crate::color_range_ui::tool_of(&f, mods).to_string();
    crate::color_range_ui::pick(app, &mut f, at, mods, "order");
    // From colour follows the first sample (the point's own colour keeps the Lab box unchanged).
    let pts = f.get("points").and_then(Value::as_array).cloned().unwrap_or_default();
    let first = pts.first().and_then(|p| Some((p.get(0)?.as_f64()?, p.get(1)?.as_f64()?)));
    if let Some((x, y)) = first
        && tool != "subtract"
        && (tool == "sample" || pts.len() == 1)
        && let Some(rgb) = crate::canvas::composite_color(app, x, y)
    {
        f.insert("color".into(), json!(crate::color_picker_ui::hex(rgb)));
    }
    if let Some(c) = f.get("color").cloned() {
        f.insert("__sampledColor".into(), c);
    }
    match app.ui.dialogs.last_mut() {
        Some(d) => {
            d.fields = f;
            true
        }
        None => false,
    }
}

/// The canvas under the dialog: the eyedropper cursor at `over` (+ / − while Shift / Alt are
/// held), and a sample at document point `at` for a press. Add and subtract take a point per
/// press; only the plain eyedropper keeps sampling while the pointer drags.
pub(crate) fn canvas(app: &mut PhotocraftApp, ctx: &egui::Context, over: egui::Pos2, at: Option<[f64; 2]>) {
    let Some(f) = app.ui.dialogs.last().map(|d| &d.fields).filter(|f| owns(f)).cloned() else { return };
    let mods = ctx.input(|i| i.modifiers);
    let tool = crate::color_range_ui::tool_of(&f, mods);
    crate::color_range_ui::eyedropper_cursor(app, ctx, over, crate::color_range_ui::tool_badge(tool));
    let (pressed, moved) = ctx.input(|i| (i.pointer.primary_pressed(), i.pointer.delta() != egui::Vec2::ZERO));
    if let Some(at) = at
        && (pressed || (tool == "sample" && moved))
        && pick_top(app, at, mods)
    {
        ctx.request_repaint();
    }
}

fn num(f: &Map<String, Value>, k: &str, d: f32) -> f32 {
    f.get(k).and_then(Value::as_f64).map_or(d, |v| v as f32)
}

fn rgb_of(f: &Map<String, Value>, k: &str) -> Option<[f32; 3]> {
    f.get(k).and_then(Value::as_str).and_then(crate::color_picker_ui::parse_hex)
}

/// Frame-start sync of the Result swatch (`__result`, a hex) with the From `color` and the Hue,
/// Saturation and Lightness sliders.
///
/// * A From colour changed outside the eyedropper (its Color Picker) no longer matches the samples
///   (`__sampledColor` is the hex `pick_top` last set), so the samples are dropped.
/// * A `__result` the Color Picker wrote (it differs from `__resultSynced`, the hex this function
///   last wrote) sets Hue, Saturation and Lightness through `replace_color_shift`. Hue follows the
///   slider's sign: red to blue is -120. The shift is whole numbers, clamped to the slider ranges,
///   so `__result` is then rewritten to what the sliders actually give.
/// * Otherwise `__result` is rewritten to what the sliders give for the current From colour.
///
/// `__result`, `__resultSynced` and `__sampledColor` start with `__`, so `params` never sends them.
pub(crate) fn sync_result(f: &mut Map<String, Value>) {
    let Some(from) = rgb_of(f, "color") else { return };
    let color = crate::color_picker_ui::hex(from);
    match f.get("__sampledColor").and_then(Value::as_str) {
        Some(sampled) if sampled != color && non_empty(f, "points") => {
            for k in ["points", "subtractPoints", "order"] {
                f.insert(k.into(), json!([]));
            }
        }
        _ => {}
    }
    f.insert("__sampledColor".into(), json!(color));

    let picked = rgb_of(f, "__result").filter(|_| f.get("__result") != f.get("__resultSynced"));
    if let Some(result) = picked {
        // `+ 0.0` turns -0.0 into 0.0.
        for (k, v) in ["hue", "saturation", "lightness"].into_iter().zip(photocraft_algo::tone::replace_color_shift(from, result)) {
            f.insert(k.into(), json!(f64::from(v) + 0.0));
        }
    }
    let result =
        crate::color_picker_ui::hex(photocraft_algo::tone::replace_color_result(from, num(f, "hue", 0.0), num(f, "saturation", 0.0), num(f, "lightness", 0.0)));
    f.insert("__result".into(), json!(result));
    f.insert("__resultSynced".into(), json!(result));
}

/// Dialog body, in Photoshop's order.
pub(crate) fn body(_app: &mut PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    sync_result(f);
    let t = Tokens::get(ui.ctx());
    let localized = f.get("localized").and_then(Value::as_bool).unwrap_or(false);
    let mut on = localized;
    if widgets::checkbox(ui, &mut on, tl!("Localized Color Clusters")).changed() {
        f.insert("localized".into(), json!(on));
    }
    ui.add_space(4.0);
    let tool = f.get("__tool").and_then(Value::as_str).unwrap_or("sample").to_string();
    let name = crate::filter_dialog::label("color");
    ui.horizontal(|ui| {
        for (id, badge, label) in [("sample", "", tl!("Eyedropper Tool")), ("add", "+", tl!("Add to Sample")), ("subtract", "−", tl!("Subtract from Sample"))]
        {
            if crate::color_range_ui::eyedropper(ui, badge, tool == id, label).clicked() {
                f.insert("__tool".into(), json!(id));
            }
        }
        ui.add_space(8.0);
        ui.label(egui::RichText::new(&name).color(t.text_dim));
        let rgb = f.get("color").and_then(Value::as_str).and_then(crate::color_picker_ui::parse_hex).unwrap_or([0.0; 3]);
        let b = rgb.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
        if widgets::color_swatch_button(ui, Color32::from_rgb(b[0], b[1], b[2]), &name).clicked() {
            crate::color_picker_ui::request_field(f, "color", &crate::color_picker_ui::title_for(&name));
        }
        ui.label(egui::RichText::new(crate::color_picker_ui::hex(rgb)).color(t.text_dim).monospace());
    });
    ui.add_space(4.0);
    let slider = |ui: &mut egui::Ui, f: &mut Map<String, Value>, key: &str, range: std::ops::RangeInclusive<f32>, default: f32| {
        let mut v = num(f, key, default);
        let g = widgets::RowGestures { reset: Some(default), wheel_step: None };
        widgets::slider_row_with(ui, &crate::filter_dialog::label(key), &mut v, range, "", None, g);
        f.insert(key.into(), json!(v.round()));
    };
    slider(ui, f, "fuzziness", 0.0..=200.0, 40.0);
    // Range only applies with Localized Color Clusters on, as in Photoshop.
    ui.add_enabled_ui(localized, |ui| slider(ui, f, "range", 0.0..=100.0, 100.0));
    // The preview thumbnail (Selection / Image) goes here.
    ui.add_space(6.0);
    slider(ui, f, "hue", -180.0..=180.0, 0.0);
    slider(ui, f, "saturation", -100.0..=100.0, 0.0);
    slider(ui, f, "lightness", -100.0..=100.0, 0.0);
    ui.horizontal(|ui| {
        let name = tl!("Result");
        ui.label(egui::RichText::new(name).color(t.text_dim));
        let rgb = rgb_of(f, "__result").unwrap_or([0.0; 3]);
        let b = rgb.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
        if widgets::color_swatch_button(ui, Color32::from_rgb(b[0], b[1], b[2]), name).clicked() {
            crate::color_picker_ui::request_field(f, "__result", &crate::color_picker_ui::title_for(name));
        }
        ui.label(egui::RichText::new(crate::color_picker_ui::hex(rgb)).color(t.text_dim).monospace());
    });
    if let Some(mut preview) = f.get("__preview").and_then(Value::as_bool) {
        ui.add_space(4.0);
        widgets::checkbox(ui, &mut preview, tl!("Preview"));
        f.insert("__preview".into(), json!(preview));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{Color, ColorMode, Document, SampleType, Size};
    use photocraft_geom::Rect as GRect;

    /// 40 × 30: red block (0..20, 0..15), blue block (20..40, 0..15), black and white below.
    fn app_with_doc() -> PhotocraftApp {
        let mut doc = Document::with_background("rc", Size::new(40, 30), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        let bg = doc.layers[0].surface_mut().unwrap();
        bg.fill_rect(GRect::new(0, 0, 20, 15), &[1.0, 0.0, 0.0, 1.0]);
        bg.fill_rect(GRect::new(20, 0, 40, 15), &[0.0, 0.0, 1.0, 1.0]);
        bg.fill_rect(GRect::new(0, 15, 20, 30), &[0.0, 0.0, 0.0, 1.0]);
        let mut s = photocraft_engine::Session::new();
        s.add_document(doc, None);
        PhotocraftApp::new(s, crate::Services::default())
    }

    fn fields(app: &PhotocraftApp) -> Map<String, Value> {
        app.ui.dialogs.last().unwrap().fields.clone()
    }

    #[test]
    fn opens_with_sample_eyedropper_armed() {
        let mut app = app_with_doc();
        crate::filter_dialog::open(&mut app, COMMAND).unwrap();
        assert!(sampling_top(&app));
        assert_eq!(app.ui.dialogs.last().unwrap().fields["__tool"], json!("sample"));
    }

    #[test]
    fn plain_click_samples_and_sets_colour() {
        let mut app = app_with_doc();
        crate::filter_dialog::open(&mut app, COMMAND).unwrap();
        assert!(pick_top(&mut app, [5.0, 5.0], egui::Modifiers::NONE));
        let f = &app.ui.dialogs.last().unwrap().fields;
        assert_eq!(f["points"], json!([[5.0, 5.0]]));
        assert_eq!(f["color"], json!("#ff0000"));
        assert_eq!(f["order"], json!(["+"]));
    }

    #[test]
    fn shift_adds_and_alt_subtracts_in_click_order() {
        let mut app = app_with_doc();
        crate::filter_dialog::open(&mut app, COMMAND).unwrap();
        pick_top(&mut app, [5.0, 5.0], egui::Modifiers::NONE);
        pick_top(&mut app, [25.0, 5.0], egui::Modifiers::SHIFT);
        pick_top(&mut app, [25.0, 5.0], egui::Modifiers::ALT);
        let f = &app.ui.dialogs.last().unwrap().fields;
        assert_eq!(f["points"].as_array().unwrap().len(), 2);
        assert_eq!(f["subtractPoints"].as_array().unwrap().len(), 1);
        assert_eq!(f["order"], json!(["+", "+", "-"]));
        assert_eq!(f["color"], json!("#ff0000"), "From colour stays the first sample");
    }

    #[test]
    fn ok_runs_one_step_with_samples() {
        let mut app = app_with_doc();
        let id = crate::filter_dialog::open(&mut app, COMMAND).unwrap();
        pick_top(&mut app, [5.0, 5.0], egui::Modifiers::NONE);
        app.ui.dialog_mut(id).unwrap().fields.insert("hue".into(), json!(120));
        let before = app.session.active().unwrap().history.past_len();
        crate::dialogs::confirm(&mut app, id).unwrap();
        assert_eq!(app.session.active().unwrap().history.past_len(), before + 1);
    }

    #[test]
    fn cancel_disarms_canvas_eyedropper() {
        let mut app = app_with_doc();
        let id = crate::filter_dialog::open(&mut app, COMMAND).unwrap();
        pick_top(&mut app, [5.0, 5.0], egui::Modifiers::NONE);
        crate::dialogs::cancel(&mut app, id).unwrap();
        assert!(!sampling_top(&app));
        assert!(!pick_top(&mut app, [5.0, 5.0], egui::Modifiers::NONE));
    }

    #[test]
    fn off_image_click_changes_nothing() {
        let mut app = app_with_doc();
        crate::filter_dialog::open(&mut app, COMMAND).unwrap();
        let before = fields(&app);
        assert!(!pick_top(&mut app, [-1.0, 5.0], egui::Modifiers::NONE));
        assert!(!pick_top(&mut app, [5.0, 99.0], egui::Modifiers::NONE));
        assert_eq!(fields(&app), before);
    }

    /// R1: the engine rejects `localized` without `points`, so the dialog never sends it that way
    /// (neither to the live preview nor to OK, which both build their params with `params_of`).
    #[test]
    fn localized_is_only_sent_with_samples() {
        let mut app = app_with_doc();
        let id = crate::filter_dialog::open(&mut app, COMMAND).unwrap();
        {
            let f = &mut app.ui.dialog_mut(id).unwrap().fields;
            f.insert("localized".into(), json!(true));
            f.insert("range".into(), json!(50));
        }
        let p = crate::filter_dialog::params_of(&fields(&app));
        for k in ["localized", "range", "points", "subtractPoints", "order"] {
            assert!(p.get(k).is_none(), "{k} must not be sent without samples: {p}");
        }
        // OK with Localized ticked and no samples runs.
        crate::dialogs::confirm(&mut app, id).unwrap();

        let id = crate::filter_dialog::open(&mut app, COMMAND).unwrap();
        app.ui.dialog_mut(id).unwrap().fields.insert("localized".into(), json!(true));
        pick_top(&mut app, [5.0, 5.0], egui::Modifiers::NONE);
        let p = crate::filter_dialog::params_of(&fields(&app));
        assert_eq!(p["localized"], json!(true));
        assert_eq!(p["points"], json!([[5.0, 5.0]]));
        assert_eq!(p["order"], json!(["+"]));
        assert!(p.get("subtractPoints").is_none());
        assert!(p.as_object().unwrap().keys().all(|k| !k.starts_with("__")));
    }

    /// The reduced live preview reads each sample from the pixel of the copy that covers it.
    #[test]
    fn preview_scales_samples_to_the_reduced_copy() {
        let app = app_with_doc();
        let mut p = json!({"points": [[25.0, 9.0]], "subtractPoints": [[39.0, 29.0]], "hue": 10});
        scale_samples_for_proxy(&mut p, &app.session, 4);
        assert_eq!(p["points"], json!([[6.0, 2.0]]));
        assert_eq!(p["subtractPoints"], json!([[9.0, 7.0]]));
        assert_eq!(p["hue"], json!(10));
    }

    #[test]
    fn result_swatch_tracks_sliders() {
        let mut app = app_with_doc();
        let id = crate::filter_dialog::open(&mut app, COMMAND).unwrap();
        pick_top(&mut app, [5.0, 5.0], egui::Modifiers::NONE);
        let f = &mut app.ui.dialog_mut(id).unwrap().fields;
        f.insert("hue".into(), json!(120));
        sync_result(f);
        assert_eq!(f["__result"], json!("#00ff00"));
    }

    /// Hue convention: the shift from red to blue is -120 (replace_color_shift's sign).
    #[test]
    fn picking_a_result_sets_hue_saturation_lightness() {
        let mut app = app_with_doc();
        let id = crate::filter_dialog::open(&mut app, COMMAND).unwrap();
        pick_top(&mut app, [5.0, 5.0], egui::Modifiers::NONE);
        let f = &mut app.ui.dialog_mut(id).unwrap().fields;
        sync_result(f);
        assert_eq!(f["__result"], json!("#ff0000"));
        f.insert("__result".into(), json!("#0000ff"));
        sync_result(f);
        assert_eq!(f["hue"].as_f64(), Some(-120.0));
        assert_eq!(f["saturation"].as_f64(), Some(0.0));
        assert_eq!(f["lightness"].as_f64(), Some(0.0));
        assert_eq!(f["__result"], json!("#0000ff"));
    }

    /// A new sample moves From colour, which must not be read as the user picking a new Result.
    #[test]
    fn resampling_keeps_the_shift_and_updates_the_result() {
        let mut app = app_with_doc();
        let id = crate::filter_dialog::open(&mut app, COMMAND).unwrap();
        pick_top(&mut app, [5.0, 5.0], egui::Modifiers::NONE);
        app.ui.dialog_mut(id).unwrap().fields.insert("hue".into(), json!(120));
        sync_result(&mut app.ui.dialog_mut(id).unwrap().fields);
        pick_top(&mut app, [25.0, 5.0], egui::Modifiers::NONE);
        let f = &mut app.ui.dialog_mut(id).unwrap().fields;
        sync_result(f);
        assert_eq!(f["hue"].as_f64(), Some(120.0));
        assert_eq!(f["points"], json!([[25.0, 5.0]]), "an eyedropper sample is not cleared");
        assert_ne!(f["__result"], json!("#00ff00"));
    }

    #[test]
    fn picking_from_colour_clears_samples() {
        let mut app = app_with_doc();
        let id = crate::filter_dialog::open(&mut app, COMMAND).unwrap();
        pick_top(&mut app, [5.0, 5.0], egui::Modifiers::NONE);
        let f = &mut app.ui.dialog_mut(id).unwrap().fields;
        f.insert("color".into(), json!("#00ff00"));
        sync_result(f);
        assert_eq!(f["points"], json!([]));
        assert_eq!(f["subtractPoints"], json!([]));
        assert_eq!(f["order"], json!([]));
    }

    /// `__result` and the sync bookkeeping never reach the engine, and OK still runs.
    #[test]
    fn result_never_reaches_the_engine() {
        let mut app = app_with_doc();
        let id = crate::filter_dialog::open(&mut app, COMMAND).unwrap();
        pick_top(&mut app, [5.0, 5.0], egui::Modifiers::NONE);
        let f = &mut app.ui.dialog_mut(id).unwrap().fields;
        sync_result(f);
        assert!(f.contains_key("__result"));
        let p = crate::filter_dialog::params_of(f);
        assert!(p.as_object().unwrap().keys().all(|k| !k.starts_with("__")), "{p}");
        crate::dialogs::confirm(&mut app, id).unwrap();
    }
}
