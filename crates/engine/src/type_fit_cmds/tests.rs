use super::*;
use photocraft_doc::text::{TextFit, TextShape};
use photocraft_doc::vector::{Path, Subpath};
use photocraft_doc::{LayerContent, LayerId, TextLayer};
use serde_json::json;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300, "depth": 8, "background": "white"})).unwrap();
    s
}

fn session_with_box_text(words: &str, size: f32, b: [f32; 4]) -> (Session, LayerId) {
    let mut s = session();
    let r = s.execute("type.create", json!({"x": 0, "y": 0, "text": words, "size": size, "color": "#000000", "box": b})).unwrap();
    let id = LayerId(r["layer"].as_u64().unwrap());
    assert!(matches!(text(&s, id).shape, TextShape::Box { .. }));
    (s, id)
}

fn session_with_point_text(words: &str, size: f32) -> (Session, LayerId) {
    let mut s = session();
    let r = s.execute("type.create", json!({"x": 100, "y": 100, "text": words, "size": size, "color": "#000000"})).unwrap();
    let id = LayerId(r["layer"].as_u64().unwrap());
    assert_eq!(text(&s, id).shape, TextShape::Point);
    (s, id)
}

fn text(s: &Session, id: LayerId) -> TextLayer {
    match &s.active().unwrap().doc.layer(id).unwrap().content {
        LayerContent::Text(t) => t.clone(),
        c => panic!("not text: {}", c.kind_name()),
    }
}

fn history_len(s: &Session) -> usize {
    s.active().unwrap().history.entries().len()
}

fn lock_all(s: &mut Session, id: LayerId) {
    s.execute("layer.setProps", json!({"layer": id.0, "locked": true})).unwrap();
}

#[test]
fn convert_fits_box_text_in_one_step() {
    let (mut s, id) = session_with_box_text("Hello dynamic world", 12.0, [0.0, 0.0, 300.0, 100.0]);
    assert!(s.is_enabled("type.convertToDynamicText"));
    assert!(!s.is_enabled("type.releaseDynamicText") && !s.is_enabled("type.setDynamicText"));
    let before = history_len(&s);
    s.execute("type.convertToDynamicText", json!({"layer": id.0})).unwrap();
    assert_eq!(history_len(&s), before + 1);
    let t = text(&s, id);
    assert!(t.fit.is_some() && t.size_pt > 12.0, "fills the big box: {}", t.size_pt);
    assert!(!s.is_enabled("type.convertToDynamicText"), "already Dynamic Text");
    assert!(s.is_enabled("type.releaseDynamicText") && s.is_enabled("type.setDynamicText"));
    // Converting twice is refused without a history step.
    assert!(s.execute("type.convertToDynamicText", json!({"layer": id.0})).is_err());
    assert_eq!(history_len(&s), before + 1);
}

#[test]
fn point_text_becomes_box_then_fits() {
    let (mut s, id) = session_with_point_text("Hi", 12.0);
    let before = history_len(&s);
    s.execute("type.convertToDynamicText", json!({"layer": id.0})).unwrap();
    assert_eq!(history_len(&s), before + 1, "box and fit in one step");
    assert!(matches!(text(&s, id).shape, TextShape::Box { .. }) && text(&s, id).fit.is_some());
    assert!(s.undo());
    assert_eq!(text(&s, id).shape, TextShape::Point);
    assert!(text(&s, id).fit.is_none());
}

#[test]
fn area_text_fits() {
    let (mut s, id) = session_with_point_text("Well, that went better than expected.", 12.0);
    s.edit("path", |doc, _| {
        doc.work_path = Some(Path::new(vec![Subpath::polygon(&[(50.0, 50.0), (350.0, 50.0), (350.0, 250.0), (50.0, 250.0)])]));
        Ok(())
    })
    .unwrap();
    s.execute("type.fillPath", json!({"layer": id.0})).unwrap();
    s.execute("type.convertToDynamicText", json!({"layer": id.0})).unwrap();
    let t = text(&s, id);
    assert!(matches!(t.shape, TextShape::Area { .. }) && t.fit.is_some());
    assert!(t.size_pt > 12.0, "fills the area: {}", t.size_pt);
}

#[test]
fn typing_refits_and_stays_bounded() {
    let (mut s, id) = session_with_box_text("Hi", 12.0, [0.0, 0.0, 200.0, 80.0]);
    s.execute("type.convertToDynamicText", json!({"layer": id.0})).unwrap();
    let big = text(&s, id).size_pt;
    s.execute("type.edit", json!({"layer": id.0, "text": "Hi ".repeat(400)})).unwrap();
    let small = text(&s, id).size_pt;
    assert!(small < big && small >= photocraft_text::fit::MIN_PT - 1e-4, "{big} -> {small}");
}

#[test]
fn resize_then_undo_restores_box_and_size() {
    let (mut s, id) = session_with_box_text("Hello there", 12.0, [0.0, 0.0, 300.0, 100.0]);
    s.execute("type.convertToDynamicText", json!({"layer": id.0})).unwrap();
    let (shape0, size0) = (text(&s, id).shape.clone(), text(&s, id).size_pt);
    let before = history_len(&s);
    s.execute("type.edit", json!({"layer": id.0, "box": [0.0, 0.0, 120.0, 100.0]})).unwrap();
    assert_eq!(history_len(&s), before + 1);
    assert!(text(&s, id).size_pt < size0);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(text(&s, id).shape, shape0);
    assert!((text(&s, id).size_pt - size0).abs() < 1e-4);
}

#[test]
fn release_keeps_sizes() {
    let (mut s, id) = session_with_box_text("Hello", 12.0, [0.0, 0.0, 300.0, 100.0]);
    s.execute("type.convertToDynamicText", json!({"layer": id.0})).unwrap();
    let size = text(&s, id).size_pt;
    s.execute("type.releaseDynamicText", json!({"layer": id.0})).unwrap();
    assert!(text(&s, id).fit.is_none() && (text(&s, id).size_pt - size).abs() < 1e-4);
    let before = history_len(&s);
    assert!(s.execute("type.releaseDynamicText", json!({"layer": id.0})).unwrap_err().to_string().contains("not Dynamic Text"));
    assert!(s.execute("type.setDynamicText", json!({"layer": id.0, "lineGap": 10})).unwrap_err().to_string().contains("not Dynamic Text"));
    assert_eq!(history_len(&s), before);
    // Released text no longer refits: a smaller box keeps the size.
    s.execute("type.edit", json!({"layer": id.0, "box": [0.0, 0.0, 60.0, 100.0]})).unwrap();
    assert!((text(&s, id).size_pt - size).abs() < 1e-4);
}

#[test]
fn gaps_validate_and_apply() {
    let (mut s, id) = session_with_box_text("a b c d e f", 12.0, [0.0, 0.0, 300.0, 300.0]);
    s.execute("type.convertToDynamicText", json!({"layer": id.0})).unwrap();
    let before = history_len(&s);
    for bad in [json!({"wordGap": "x"}), json!({"lineGap": 1000}), json!({"wordGap": -51}), json!({"lineGap": true}), json!({"wordGap": 200.5})] {
        let mut p = bad.clone();
        p["layer"] = json!(id.0);
        let e = s.execute("type.setDynamicText", p).unwrap_err();
        assert!(matches!(e, EngineError::BadParams { ref cmd, .. } if cmd == "type.setDynamicText"), "{bad}: {e}");
    }
    assert_eq!(history_len(&s), before);
    assert_eq!(text(&s, id).fit, Some(TextFit::default()));
    s.execute("type.setDynamicText", json!({"layer": id.0, "lineGap": 50})).unwrap();
    assert_eq!(history_len(&s), before + 1);
    assert_eq!(text(&s, id).fit.unwrap().line_gap, 50.0);
    assert_eq!(text(&s, id).fit.unwrap().word_gap, 0.0, "unspecified gap kept");
    s.execute("type.setDynamicText", json!({"layer": id.0, "wordGap": -50, "lineGap": 200})).unwrap();
    assert_eq!(text(&s, id).fit, Some(TextFit { word_gap: -50.0, line_gap: 200.0 }));
}

#[test]
fn bigger_line_gap_shrinks_the_fit() {
    let words = "one two three four five six seven eight nine ten eleven twelve";
    let (mut s, id) = session_with_box_text(words, 12.0, [0.0, 0.0, 200.0, 200.0]);
    s.execute("type.convertToDynamicText", json!({"layer": id.0})).unwrap();
    let size0 = text(&s, id).size_pt;
    s.execute("type.setDynamicText", json!({"layer": id.0, "lineGap": 200})).unwrap();
    assert!(text(&s, id).size_pt < size0, "{size0} -> {}", text(&s, id).size_pt);
}

#[test]
fn refused_targets() {
    // Vertical.
    let (mut s, id) = session_with_box_text("abc", 12.0, [0.0, 0.0, 100.0, 100.0]);
    s.execute("type.orientation.vertical", json!({"layer": id.0})).unwrap();
    assert!(!s.is_enabled("type.convertToDynamicText"));
    let before = history_len(&s);
    let e = s.execute("type.convertToDynamicText", json!({"layer": id.0})).unwrap_err().to_string();
    assert!(e.contains("vertical"), "{e}");
    assert_eq!(history_len(&s), before);
    assert!(text(&s, id).fit.is_none());

    // Right-to-left paragraphs.
    let (mut s, id) = session_with_box_text("abc", 12.0, [0.0, 0.0, 100.0, 100.0]);
    s.execute("type.setStyle", json!({"layer": id.0, "direction": "rtl"})).unwrap();
    assert!(!s.is_enabled("type.convertToDynamicText"));
    let e = s.execute("type.convertToDynamicText", json!({"layer": id.0})).unwrap_err().to_string();
    assert!(e.contains("right-to-left"), "{e}");
    assert!(text(&s, id).fit.is_none());

    // Path text.
    let (mut s, id) = session_with_point_text("abc", 12.0);
    s.edit("path", |doc, _| {
        doc.work_path = Some(Path::new(vec![Subpath::polyline(&[(10.0, 200.0), (390.0, 200.0)])]));
        Ok(())
    })
    .unwrap();
    s.execute("type.attachToPath", json!({"layer": id.0})).unwrap();
    assert!(matches!(text(&s, id).shape, TextShape::Path { .. }));
    assert!(!s.is_enabled("type.convertToDynamicText"));
    let e = s.execute("type.convertToDynamicText", json!({"layer": id.0})).unwrap_err().to_string();
    assert!(e.contains("path"), "{e}");
    assert!(text(&s, id).fit.is_none());

    // Several layers at once.
    let (mut s, a) = session_with_box_text("abc", 12.0, [0.0, 0.0, 100.0, 100.0]);
    let b = LayerId(s.execute("type.create", json!({"x": 0, "y": 0, "text": "def", "box": [0, 120, 100, 100]})).unwrap()["layer"].as_u64().unwrap());
    let e = s.execute("type.convertToDynamicText", json!({"layers": [a.0, b.0]})).unwrap_err().to_string();
    assert!(e.contains("one type layer"), "{e}");
    assert!(text(&s, a).fit.is_none() && text(&s, b).fit.is_none());

    // Not a type layer.
    let mut s = session();
    let bg = s.active().unwrap().active_layer.unwrap();
    assert!(!s.is_enabled("type.convertToDynamicText"));
    assert!(s.execute("type.convertToDynamicText", json!({"layer": bg.0})).is_err());
    for p in [json!({"layer": "x"}), json!({"layer": u64::MAX})] {
        assert!(s.execute("type.convertToDynamicText", p.clone()).is_err());
        assert!(s.execute("type.releaseDynamicText", p.clone()).is_err());
        assert!(s.execute("type.setDynamicText", p).is_err());
    }
}

#[test]
fn turning_a_fitted_layer_vertical_stops_refitting() {
    let (mut s, id) = session_with_box_text("Hello there", 12.0, [0.0, 0.0, 300.0, 100.0]);
    s.execute("type.convertToDynamicText", json!({"layer": id.0})).unwrap();
    s.execute("type.orientation.vertical", json!({"layer": id.0})).unwrap();
    let size = text(&s, id).size_pt;
    s.execute("type.edit", json!({"layer": id.0, "text": "Hello there ".repeat(50)})).unwrap();
    assert!((text(&s, id).size_pt - size).abs() < 1e-4, "the solver never sees vertical text");
}

#[test]
fn locked_layer_is_refused() {
    let (mut s, id) = session_with_box_text("abc", 12.0, [0.0, 0.0, 100.0, 100.0]);
    lock_all(&mut s, id);
    assert!(s.execute("type.convertToDynamicText", json!({"layer": id.0})).unwrap_err().to_string().contains("locked"));
    assert!(text(&s, id).fit.is_none());
}
