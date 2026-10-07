//! Right-clicking a typeset equation offers **Edit Equation**, which reopens the equation dialog on
//! that equation's source; Replace writes the edit back as one equation.
//!
//! The app runs in a headless `egui_kittest` harness and is driven through the control channel —
//! the same surface `examples/ui_shot.rs` and agents use.

use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};
use wordcraft_doc::Pos;
use wordcraft_doc::para::InlineObject;
use wordcraft_engine::Session;
use wordcraft_ui_egui::dialogs::Dialog;
use wordcraft_ui_egui::{ControlRequest, Services, WordApp};

type Fallible = Result<(), Box<dyn std::error::Error>>;

/// The app in a harness, plus the control channel that drives it.
struct Ui {
    harness: egui_kittest::Harness<'static, WordApp>,
    tx: std::sync::mpsc::Sender<ControlRequest>,
}

impl Ui {
    fn new() -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let app = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default()).with_control(rx);
        let harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1000.0, 700.0))
            .with_pixels_per_point(1.0)
            .with_max_steps(1_000_000)
            .build_ui_state(
                |ui, app: &mut WordApp| {
                    let ctx = ui.ctx().clone();
                    app.logic(&ctx);
                    app.ui(ui);
                },
                app,
            );
        let mut ui = Ui { harness, tx };
        ui.harness.input_mut().max_texture_side = Some(8192);
        ui.steps(6); // fonts, layout, first paint
        ui
    }

    fn step(&mut self) {
        let mut raw = std::mem::take(self.harness.input_mut());
        self.harness.state_mut().raw_input_hook(&mut raw);
        *self.harness.input_mut() = raw;
        self.harness.step();
    }

    fn steps(&mut self, n: usize) {
        for _ in 0..n {
            self.step();
        }
    }

    /// Send a control request and return its result once the app has answered and the input it
    /// queued (one event per frame) has landed.
    fn cmd(&mut self, method: &str, params: Value) -> Result<Value, Box<dyn std::error::Error>> {
        let (req, reply) = ControlRequest::new(method, params);
        self.tx.send(req)?;
        for _ in 0..60 {
            self.step();
            if let Ok(v) = reply.try_recv() {
                self.steps(3);
                return match v.get("ok").and_then(Value::as_bool) {
                    Some(true) => Ok(v.get("result").cloned().unwrap_or(Value::Null)),
                    _ => Err(format!("{method}: {}", v.get("error").and_then(Value::as_str).unwrap_or("failed")).into()),
                };
            }
        }
        Err(format!("no reply to {method}").into())
    }

    fn app(&mut self) -> &mut WordApp {
        self.harness.state_mut()
    }

    /// Screen position (logical points) just inside the first inline object of the caret's line.
    fn equation_screen(&mut self) -> Option<egui::Pos2> {
        let app = self.harness.state_mut();
        let caret = app.session.sel.focus.clone();
        let before_object = Pos { off: 0, ..caret };
        let c = app.session.layout().caret_on(&before_object, 0)?;
        wordcraft_ui_egui::canvas::page_to_screen(app, c.page, c.x + 3.0, c.top + c.height * 0.5)
    }
}

#[test]
fn right_clicking_an_equation_edits_its_source() -> Fallible {
    let mut ui = Ui::new();
    ui.cmd("engine.execute", json!({"command": "insert.equation", "params": {"latex": "\\frac{a}{b}"}}))?;

    // Right-click the typeset equation: the menu offers to edit it.
    let at = ui.equation_screen().ok_or("the equation is not on screen")?;
    ui.cmd("ui.click", json!({"x": at.x, "y": at.y, "button": "right"}))?;
    ui.steps(3);
    ui.harness.get_by_label_contains("Edit Equation").click();
    ui.steps(3);

    // The dialog opened on the source of the equation that was clicked.
    let seeded = match ui.app().dialog.as_ref() {
        Some(Dialog::Equation { latex, display, .. }) => Some((latex.clone(), *display)),
        _ => None,
    };
    let (latex, display) = seeded.ok_or("expected the equation dialog")?;
    assert_eq!(latex, "\\frac{a}{b}");
    assert!(!display);

    // Click into the dialog's field, select all, and type a new source.
    {
        let field = ui.harness.get_by_role(egui::accesskit::Role::MultilineTextInput);
        assert_eq!(field.value().as_deref(), Some("\\frac{a}{b}"), "the field is seeded with the source");
        field.click();
    }
    ui.steps(2);
    ui.harness.input_mut().events.push(egui::Event::Key {
        key: egui::Key::A,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    });
    ui.steps(2);
    ui.harness.get_by_role(egui::accesskit::Role::MultilineTextInput).type_text("\\sqrt{x}");
    ui.steps(2);
    ui.harness.get_by_label_contains("Replace").click();
    ui.steps(3);

    // The document holds the edited source, as one equation (not a second one beside the first).
    let r = ui.cmd("engine.execute", json!({"command": "equation.source"}))?;
    assert_eq!(r.get("latex").and_then(Value::as_str), Some("\\sqrt{x}"));
    let equations = {
        let app = ui.app();
        let caret = app.session.sel.focus.clone();
        let p = app.session.doc.para(caret.story, &caret.path).ok_or("the caret's paragraph")?;
        p.object_offsets().into_iter().filter(|off| matches!(p.object_at(*off), Some(InlineObject::Equation { .. }))).count()
    };
    assert_eq!(equations, 1, "Replace rewrites the equation rather than adding another");

    // On plain text the menu still opens, but without the equation item.
    ui.cmd("engine.execute", json!({"command": "text.insert", "params": {"text": " plain"}}))?;
    let at = wordcraft_ui_egui::canvas::caret_screen(ui.app()).ok_or("caret off screen")?.0;
    ui.cmd("ui.click", json!({"x": at.x, "y": at.y, "button": "right"}))?;
    ui.steps(3);
    ui.harness.get_by_label_contains("Paste"); // the menu is open …
    assert!(ui.harness.query_by_label_contains("Edit Equation").is_none(), "no equation under the caret, so no Edit Equation");
    Ok(())
}
