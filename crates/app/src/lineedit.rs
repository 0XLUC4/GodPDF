use egui::{
    Color32, FontFamily, FontId, Frame, Id, Key, Margin, Rect, TextEdit, Ui, UiBuilder, vec2,
};

pub struct LineEdit {
    pub tab: u64,
    pub page: usize,
    pub band: Rect,
    pub area: godpdf_core::Rect,
    pub text: String,
    original: String,
    size: f32,
    color: Color32,
    focus_pending: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Commit,
    Cancel,
}

impl LineEdit {
    pub fn new(
        tab: u64,
        page: usize,
        band: Rect,
        area: godpdf_core::Rect,
        text: String,
        look: godpdf_core::LineLook,
    ) -> Self {
        let color = look.color.map_or(Color32::BLACK, |color| {
            Color32::from_rgb(
                (color.r * 255.0).round() as u8,
                (color.g * 255.0).round() as u8,
                (color.b * 255.0).round() as u8,
            )
        });
        Self {
            tab,
            page,
            band,
            area,
            original: text.clone(),
            text,
            size: look.size,
            color,
            focus_pending: true,
        }
    }

    pub fn changed(&self) -> bool {
        self.text != self.original
    }

    fn id(&self) -> Id {
        Id::new(("line-edit", self.tab))
    }
}

pub fn show(ui: &mut Ui, edit: &mut LineEdit, page_rect: Rect, zoom: f32) -> Option<Event> {
    let id = edit.id();
    let font = FontId::new(
        (edit.size * zoom).max(1.0),
        FontFamily::Name(
            format!(
                "pdf-{}",
                godpdf_core::Font::Helvetica.resource(false, false)
            )
            .into(),
        ),
    );
    let band = Rect::from_min_size(
        page_rect.min + edit.band.min.to_vec2() * zoom,
        edit.band.size() * zoom,
    );
    let galley = ui
        .painter()
        .layout_no_wrap(edit.text.clone(), font.clone(), edit.color);
    let width = band.width().max(galley.size().x + font.size);
    let height = band.height().max(galley.size().y);
    let field = Rect::from_min_size(
        band.left_center() - vec2(0.0, height / 2.0),
        vec2(width, height),
    );
    let cover = field.union(band).expand(2.0);
    ui.painter().rect_filled(cover, 2.0, Color32::WHITE);
    ui.painter().rect_stroke(
        cover,
        2.0,
        egui::Stroke::new(1.0, Color32::from_rgb(0x25, 0x63, 0xEB)),
        egui::StrokeKind::Outside,
    );

    let focused = ui.memory(|memory| memory.has_focus(id));
    let (enter, escape) = ui.input_mut(|input| {
        (
            focused && input.consume_key(egui::Modifiers::NONE, Key::Enter),
            input.key_pressed(Key::Escape),
        )
    });
    let mut child = ui.new_child(UiBuilder::new().max_rect(field).id_salt(id.with("field")));
    let response = child.add(
        TextEdit::singleline(&mut edit.text)
            .id(id)
            .font(font)
            .text_color(edit.color)
            .frame(Frame::NONE)
            .margin(Margin::ZERO)
            .desired_width(width),
    );
    if std::mem::take(&mut edit.focus_pending) {
        response.request_focus();
    }
    if escape {
        return Some(Event::Cancel);
    }
    if enter {
        return Some(Event::Commit);
    }
    let clicked_outside = ui.input(|input| {
        input
            .pointer
            .any_click()
            .then(|| input.pointer.interact_pos())
            .flatten()
    });
    clicked_outside
        .is_some_and(|pos| {
            ui.clip_rect().contains(pos)
                && !cover.contains(pos)
                && ui.ctx().layer_id_at(pos) == Some(ui.layer_id())
        })
        .then_some(Event::Commit)
}
