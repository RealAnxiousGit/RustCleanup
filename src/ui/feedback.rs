//! The Compose Feedback window: a bug report or suggestion with a title, a
//! body that takes pasted screenshots and hyperlinks, and a right-click menu.
//!
//! Sending never uses a token: it opens GitHub's "New issue" page in the
//! user's browser, as the signed-in user, with the title and body filled in.
//! Images can't travel in a URL, so they are saved as PNG files in a folder
//! that opens next to the browser, ready to drag into the issue. Email is
//! only the fallback, for when the browser can't be opened or the user has
//! no GitHub account.

use std::ops::Range;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use eframe::egui::text::{CCursor, CCursorRange, LayoutJob, TextFormat};
use eframe::egui::{
    self, Color32, ColorImage, Id, Margin, RichText, Stroke, TextureHandle, Ui, ViewportCommand,
};

use super::theme::{self, Palette, semibold};
use super::widgets;
use crate::events::Logger;
use crate::i18n::{tr, trf};
use crate::sys::shell;

pub const WINDOW: &str = "feedback_window";
const NEW_ISSUE: &str = "https://github.com/RealAnxiousGit/RustCleanup/issues/new";
const EMAIL: &str = "nickbinbusiness@outlook.com";
/// Browsers and GitHub cope with URLs well past this, but not unlimited ones.
const MAX_ISSUE_URL: usize = 7000;
/// Mail programs are stricter.
const MAX_MAILTO: usize = 1800;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Kind {
    #[default]
    Bug,
    Suggestion,
}

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Kind::Bug => tr("Bug Report"),
            Kind::Suggestion => tr("Suggestion"),
        }
    }
    fn title_hint(self) -> &'static str {
        match self {
            Kind::Bug => tr("Report a bug..."),
            Kind::Suggestion => tr("Suggest a feature..."),
        }
    }
    fn body_hint(self) -> &'static str {
        match self {
            Kind::Bug => tr(
                "What happened, and what did you expect? Paste screenshots with Ctrl+V.",
            ),
            Kind::Suggestion => tr(
                "What would you like RustCleanup to do? Paste screenshots with Ctrl+V.",
            ),
        }
    }
    /// GitHub's default labels.
    fn label_name(self) -> &'static str {
        match self {
            Kind::Bug => "bug",
            Kind::Suggestion => "enhancement",
        }
    }
    fn prefix(self) -> &'static str {
        match self {
            Kind::Bug => "[Bug]",
            Kind::Suggestion => "[Suggestion]",
        }
    }
}

/// A hyperlink over a run of the body, in character indices.
#[derive(Clone, Debug, PartialEq)]
struct Link {
    range: Range<usize>,
    url: String,
}

struct Image {
    number: usize,
    image: Arc<ColorImage>,
    texture: Option<TextureHandle>,
}

impl Image {
    fn placeholder(&self) -> String {
        format!("[{}]", trf("Image {0}", &[&self.number]))
    }
}

/// Which text field a right-click menu action applies to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Title,
    Body,
}

impl Field {
    fn id(self) -> Id {
        match self {
            Field::Title => Id::unique("feedback_title"),
            Field::Body => Id::unique("feedback_body"),
        }
    }
}

struct HyperlinkDialog {
    text: String,
    url: String,
    /// The body run it replaces (the selection when it was opened).
    range: Range<usize>,
    focused: bool,
}

/// What happened when the report was sent, for the confirmation page.
struct Sent {
    email: bool,
    folder: Option<PathBuf>,
    on_clipboard: bool,
    /// Why no email draft could open, or a warning that it may not.
    mail_problem: Option<MailProblem>,
}

/// What the email app check found before a draft was opened.
#[derive(Clone)]
enum MailProblem {
    /// Nothing opens email links, so no draft was opened.
    Missing(String),
    /// Email links open somewhere that may not show a draft.
    Warning(String),
}

#[derive(Default)]
struct Draft {
    kind: Kind,
    title: String,
    body: String,
    links: Vec<Link>,
    images: Vec<Image>,
    next_image: usize,
    /// Selection when the right-click menu opened, per field.
    menu_range: Option<(Field, Range<usize>)>,
    /// Each field's selection before the latest right-click (title, body).
    /// egui moves the caret on any button press, so the menu reads this.
    held: [Range<usize>; 2],
    /// A paste asked for from the menu, done once the field has focus again.
    pending_paste: Option<(Field, Range<usize>)>,
    hyperlink: Option<HyperlinkDialog>,
    sent: Option<Sent>,
}

pub struct Feedback {
    open: Arc<AtomicBool>,
    draft: Arc<Mutex<Draft>>,
    log: Logger,
}

impl Feedback {
    pub fn new(log: Logger) -> Self {
        Self {
            open: Arc::default(),
            draft: Arc::default(),
            log,
        }
    }

    pub fn open(&self, ctx: &egui::Context) {
        if self.open.swap(true, Ordering::Relaxed) {
            super::focus_window(ctx, WINDOW);
        }
    }

    /// Keeps the window alive. Called on every pass of the main window.
    pub fn show(&self, ctx: &egui::Context) {
        if !self.open.load(Ordering::Relaxed) {
            return;
        }
        let open = self.open.clone();
        let draft = self.draft.clone();
        let log = self.log.clone();
        ctx.show_viewport_deferred(
            super::window_id(WINDOW),
            super::window_builder(tr("Compose Feedback"), [620.0, 640.0], [460.0, 440.0]),
            move |ui, _class| {
                let pal = theme::palette();
                let mut d = draft.lock().unwrap_or_else(|e| e.into_inner());
                // The hyperlink popup takes Escape for itself.
                let mut close = d.hyperlink.is_none() && super::close_pressed(ui.ctx());
                egui::CentralPanel::default()
                    .frame(
                        egui::Frame::new()
                            .fill(pal.bg)
                            .inner_margin(Margin::same(22)),
                    )
                    .show(ui, |ui| {
                        if d.sent.is_some() {
                            sent_page(ui, &pal, &mut d, &mut close);
                        } else {
                            compose(ui, &pal, &log, &mut d, &mut close);
                        }
                    });
                hyperlink_dialog(ui.ctx(), &pal, &mut d);
                if close {
                    open.store(false, Ordering::Relaxed);
                    super::closed(ui.ctx());
                }
            },
        );
    }
}

fn compose(ui: &mut Ui, pal: &Palette, log: &Logger, d: &mut Draft, close: &mut bool) {
    let ctx = ui.ctx().clone();
    take_pasted_images(&ctx, d);

    ui.label(RichText::new(tr("Compose Feedback")).family(semibold()).size(20.0));
    ui.add_space(10.0);

    egui::ComboBox::from_id_salt("feedback_kind")
        .selected_text(d.kind.label())
        .width(180.0)
        .show_ui(ui, |ui| {
            for kind in [Kind::Bug, Kind::Suggestion] {
                ui.selectable_value(&mut d.kind, kind, kind.label());
            }
        });
    ui.add_space(8.0);

    // Title
    run_pending_paste(&ctx, d, Field::Title);
    let title = egui::TextEdit::singleline(&mut d.title)
        .id(Field::Title.id())
        .hint_text(d.kind.title_hint())
        .font(egui::FontId::new(17.0, semibold()))
        .frame(egui::Frame::NONE)
        .desired_width(f32::INFINITY)
        .show(ui);
    track_right_click(&ctx, d, Field::Title, &title);
    title.response.response.context_menu(|ui| edit_menu(ui, &ctx, d, Field::Title));

    widgets::divider(ui);

    // Body, with the buttons and image strip kept below it.
    let reserved = if d.images.is_empty() { 100.0 } else { 240.0 };
    let body_height = (ui.available_height() - reserved).max(120.0);
    run_pending_paste(&ctx, d, Field::Body);
    let before = d.body.clone();
    let links = d.links.clone();
    // Links are always blue, whatever the theme's accent is.
    let link_color = if ui.visuals().dark_mode {
        egui::Color32::from_rgb(0x5a, 0xa9, 0xff)
    } else {
        egui::Color32::from_rgb(0x1a, 0x5f, 0xd6)
    };
    let mut layouter = move |ui: &Ui, text: &dyn egui::TextBuffer, wrap: f32| {
        let job = body_layout(ui, text.as_str(), &links, link_color, wrap);
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let body = egui::ScrollArea::vertical()
        .id_salt("feedback_body_scroll")
        .max_height(body_height)
        .min_scrolled_height(body_height)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::TextEdit::multiline(&mut d.body)
                .id(Field::Body.id())
                .hint_text(d.kind.body_hint())
                .frame(egui::Frame::NONE)
                .desired_width(f32::INFINITY)
                .desired_rows(8)
                .layouter(&mut layouter)
                .show(ui)
        })
        .inner;
    if d.body != before {
        let (old, new) = (before, d.body.clone());
        shift_links(&mut d.links, &old, &new);
    }
    let resp = &body.response.response;
    let link_under = |pos: Option<egui::Pos2>| {
        let pos = pos?;
        if !body.galley.rect.translate(body.galley_pos.to_vec2()).contains(pos) {
            return None;
        }
        let index = body.galley.cursor_from_pos(pos - body.galley_pos).index.0;
        d.links
            .iter()
            .find(|l| l.range.start <= index && index < l.range.end.max(l.range.start + 1))
            .cloned()
    };
    if let Some(link) = link_under(resp.hover_pos()) {
        ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
        egui::Tooltip::always_open(
            ctx.clone(),
            ui.layer_id(),
            Id::unique("feedback_link_tip"),
            egui::PopupAnchor::Pointer,
        )
        .show(|ui| ui.label(&link.url));
    }
    if resp.clicked()
        && body.cursor_range.is_none_or(|c| c.is_empty())
        && let Some(link) = link_under(resp.interact_pointer_pos())
    {
        open_link(log, &link.url);
    }
    track_right_click(&ctx, d, Field::Body, &body);
    resp.context_menu(|ui| edit_menu(ui, &ctx, d, Field::Body));

    // Pasted images
    if !d.images.is_empty() {
        widgets::divider(ui);
        image_strip(ui, pal, d);
    }

    ui.add_space(10.0);
    let ready = !d.title.trim().is_empty() && !d.body.trim().is_empty();
    ui.horizontal(|ui| {
        if widgets::primary_button(ui, pal, tr("Send"), ready)
            .on_hover_text(tr("Opens GitHub in your browser with the report filled in"))
            .clicked()
        {
            send(&ctx, log, d, false);
        }
        if widgets::button(ui, tr("Cancel"), true).clicked() {
            *close = true;
        }
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        widgets::note(ui, pal, tr("No GitHub account?"));
        let email = ui.add_enabled(ready, egui::Link::new(tr("Send it by email instead")));
        if email.clicked() {
            send(&ctx, log, d, true);
        }
    });
}

/// The confirmation shown after Send.
fn sent_page(ui: &mut Ui, pal: &Palette, d: &mut Draft, close: &mut bool) {
    let Some(sent) = &d.sent else { return };
    if let Some(MailProblem::Missing(why)) = &sent.mail_problem {
        ui.label(RichText::new(tr("The email draft could not open")).family(semibold()).size(20.0));
        ui.add_space(10.0);
        ui.label(why.as_str());
        ui.add_space(12.0);
        let mut back = false;
        ui.horizontal(|ui| {
            if widgets::primary_button(ui, pal, tr("Back to the report"), true).clicked() {
                back = true;
            }
            if widgets::button(ui, tr("Close"), true).clicked() {
                *close = true;
            }
        });
        if back {
            d.sent = None;
        } else if *close {
            *d = Draft::default();
        }
        return;
    }
    ui.label(RichText::new(tr("Thanks for the feedback")).family(semibold()).size(20.0));
    ui.add_space(10.0);
    let warning = match &sent.mail_problem {
        Some(MailProblem::Warning(w)) => Some(w.as_str()),
        _ => None,
    };
    let lines = [
        Some(if sent.email {
            tr("Your mail program opened with the report filled in. Press Send there to finish.")
        } else {
            tr("Your browser opened GitHub with the report filled in. Sign in if asked, then press Create to finish.")
        }),
        warning,
        sent.on_clipboard.then(|| {
            tr("The report was too long for a link, so it was copied to your clipboard. Paste it into the message with Ctrl+V.")
        }),
        sent.folder.is_some().then(|| {
            if sent.email {
                tr("Your screenshots were saved to the folder that opened. Attach them to the email.")
            } else {
                tr("Your screenshots were saved to the folder that opened. Drag them into the issue where it says [Image 1] and so on.")
            }
        }),
    ];
    for line in lines.into_iter().flatten() {
        ui.label(line);
        ui.add_space(6.0);
    }
    if let Some(folder) = &sent.folder {
        widgets::note(ui, pal, folder.display().to_string());
    }
    ui.add_space(12.0);
    let mut again = false;
    ui.horizontal(|ui| {
        if widgets::primary_button(ui, pal, tr("Close"), true).clicked() {
            *close = true;
        }
        if widgets::button(ui, tr("Write another"), true).clicked() {
            again = true;
        }
    });
    if again || *close {
        *d = Draft::default();
    }
}

// ---------------------------------------------------------------- editing

/// Character range of a field's selection (or the caret).
fn selection(range: &Option<CCursorRange>, text: &str) -> Range<usize> {
    let len = text.chars().count();
    match range {
        Some(r) => {
            let r = r.as_sorted_char_range();
            r.start.0.min(len)..r.end.0.min(len)
        }
        None => len..len,
    }
}

fn byte_at(text: &str, chars: usize) -> usize {
    text.char_indices().nth(chars).map_or(text.len(), |(b, _)| b)
}

fn slice(text: &str, range: &Range<usize>) -> String {
    text[byte_at(text, range.start)..byte_at(text, range.end)].to_string()
}

/// Keeps the selection through a right-click. egui moves the caret to the
/// pointer on any button press, which would drop the selection before the
/// menu opens. So: remember the selection while no right-click is under way,
/// and on the right-click put it back if the click was inside it, or move the
/// caret to the click otherwise (so Paste and Hyperlink land there).
fn track_right_click(
    ctx: &egui::Context,
    d: &mut Draft,
    field: Field,
    out: &egui::text_edit::TextEditOutput,
) {
    let slot = field as usize;
    let resp = &out.response.response;
    let right_busy = ctx.input(|i| {
        i.pointer.secondary_down() || i.pointer.secondary_pressed() || i.pointer.secondary_released()
    });
    if !right_busy {
        let now = selection(&out.cursor_range, field_text(d, field));
        d.held[slot] = now;
        return;
    }
    if !resp.secondary_clicked() {
        return;
    }
    let held = d.held[slot].clone();
    let index = resp
        .interact_pointer_pos()
        .map(|pos| out.galley.cursor_from_pos(pos - out.galley_pos).index.0);
    let range = match index {
        Some(i) if held.is_empty() || i < held.start || i > held.end => i..i,
        _ => held,
    };
    set_cursor(ctx, field, range.clone());
    d.menu_range = Some((field, range));
}

fn set_cursor(ctx: &egui::Context, field: Field, range: Range<usize>) {
    let id = field.id();
    let mut state = egui::text_edit::TextEditState::load(ctx, id).unwrap_or_default();
    state
        .cursor
        .set_char_range(Some(CCursorRange::two(CCursor::new(range.start), CCursor::new(range.end))));
    state.store(ctx, id);
    ctx.memory_mut(|m| m.request_focus(id));
}

fn field_text(d: &mut Draft, field: Field) -> &mut String {
    match field {
        Field::Title => &mut d.title,
        Field::Body => &mut d.body,
    }
}

/// Replaces a run of a field's text, keeping body links in place.
fn replace(d: &mut Draft, field: Field, range: Range<usize>, with: &str) {
    let text = field_text(d, field);
    let old = text.clone();
    let (a, b) = (byte_at(&old, range.start), byte_at(&old, range.end));
    text.replace_range(a..b, with);
    if field == Field::Body {
        let new = d.body.clone();
        shift_links(&mut d.links, &old, &new);
    }
}

/// Moves, grows, shrinks or drops links after the body changed from `old`
/// to `new`. Typing right at either end of a link stays outside it.
fn shift_links(links: &mut Vec<Link>, old: &str, new: &str) {
    let o: Vec<char> = old.chars().collect();
    let n: Vec<char> = new.chars().collect();
    let prefix = o.iter().zip(&n).take_while(|(a, b)| a == b).count();
    let max_suffix = o.len().min(n.len()) - prefix;
    let suffix = o
        .iter()
        .rev()
        .zip(n.iter().rev())
        .take(max_suffix)
        .take_while(|(a, b)| a == b)
        .count();
    let old_end = o.len() - suffix;
    let new_end = n.len() - suffix;
    for link in links.iter_mut() {
        let Range { start, end } = link.range.clone();
        if end <= prefix {
            continue;
        }
        if start >= old_end {
            link.range = (start + new_end - old_end)..(end + new_end - old_end);
            continue;
        }
        let new_start = if start < prefix { start } else { new_end };
        let new_stop = if end > old_end {
            end + new_end - old_end
        } else {
            prefix
        };
        link.range = new_start..new_stop.max(new_start);
    }
    links.retain(|l| !l.range.is_empty());
}

/// Lays the body out with its links blue and underlined.
fn body_layout(ui: &Ui, text: &str, links: &[Link], link_color: Color32, wrap: f32) -> LayoutJob {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let plain = TextFormat::simple(font.clone(), ui.visuals().text_color());
    let linked = TextFormat {
        color: link_color,
        underline: Stroke::new(1.0, link_color),
        ..TextFormat::simple(font, link_color)
    };
    let mut sorted: Vec<&Link> = links.iter().collect();
    sorted.sort_by_key(|l| l.range.start);
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap;
    let mut at = 0usize;
    let len = text.chars().count();
    for link in sorted {
        let start = link.range.start.clamp(at, len);
        let end = link.range.end.clamp(start, len);
        if start > at {
            job.append(&slice(text, &(at..start)), 0.0, plain.clone());
        }
        if end > start {
            job.append(&slice(text, &(start..end)), 0.0, linked.clone());
        }
        at = end;
    }
    if at < len || job.sections.is_empty() {
        job.append(&slice(text, &(at..len)), 0.0, plain);
    }
    job
}

/// The right-click menu for the title and the body.
fn edit_menu(ui: &mut Ui, ctx: &egui::Context, d: &mut Draft, field: Field) {
    let len = field_text(d, field).chars().count();
    let range = match &d.menu_range {
        Some((f, r)) if *f == field => r.start.min(len)..r.end.min(len),
        _ => len..len,
    };
    let has_selection = !range.is_empty();
    if ui.add_enabled(has_selection, egui::Button::new(tr("Cut"))).clicked() {
        ctx.copy_text(slice(field_text(d, field), &range));
        replace(d, field, range.clone(), "");
        set_cursor(ctx, field, range.start..range.start);
        ui.close();
    }
    if ui.add_enabled(has_selection, egui::Button::new(tr("Copy"))).clicked() {
        ctx.copy_text(slice(field_text(d, field), &range));
        set_cursor(ctx, field, range.clone());
        ui.close();
    }
    if ui.button(tr("Paste")).clicked() {
        d.pending_paste = Some((field, range.clone()));
        ui.close();
    }
    if ui
        .add_enabled(has_selection, egui::Button::new(tr("Delete")))
        .clicked()
    {
        replace(d, field, range.clone(), "");
        set_cursor(ctx, field, range.start..range.start);
        ui.close();
    }
    ui.separator();
    if ui.add_enabled(len > 0, egui::Button::new(tr("Select All"))).clicked() {
        set_cursor(ctx, field, 0..len);
        ui.close();
    }
    if field == Field::Body {
        ui.separator();
        let existing = d
            .links
            .iter()
            .position(|l| l.range.start < range.end.max(range.start + 1) && range.start < l.range.end);
        if ui.button(tr("Hyperlink")).clicked() {
            let (range, url) = match existing {
                Some(i) => (d.links[i].range.clone(), d.links[i].url.clone()),
                None => (range.clone(), String::new()),
            };
            d.hyperlink = Some(HyperlinkDialog {
                text: slice(&d.body, &range),
                url,
                range,
                focused: false,
            });
            ui.close();
        }
        if let Some(i) = existing
            && ui.button(tr("Remove Hyperlink")).clicked()
        {
            d.links.remove(i);
            ui.close();
        }
    }
}

/// Finishes a menu Paste: give the field focus at the remembered spot, then
/// ask the window for the clipboard. Text arrives as a normal paste; an
/// image arrives as `PasteImage` and becomes an attachment.
fn run_pending_paste(ctx: &egui::Context, d: &mut Draft, field: Field) {
    if let Some((f, range)) = &d.pending_paste
        && *f == field
    {
        set_cursor(ctx, field, range.clone());
        ctx.send_viewport_cmd(ViewportCommand::RequestPaste);
        d.pending_paste = None;
    }
}

/// Screenshots pasted with Ctrl+V (or the menu) go into the body as
/// "[Image N]" at the caret, with a thumbnail below.
fn take_pasted_images(ctx: &egui::Context, d: &mut Draft) {
    let pasted: Vec<Arc<ColorImage>> = ctx.input(|i| {
        i.events
            .iter()
            .filter_map(|e| match e {
                egui::Event::PasteImage(image) => Some(image.clone()),
                _ => None,
            })
            .collect()
    });
    for image in pasted {
        d.next_image += 1;
        let item = Image {
            number: d.next_image,
            image,
            texture: None,
        };
        let len = d.body.chars().count();
        let at = egui::text_edit::TextEditState::load(ctx, Field::Body.id())
            .and_then(|s| s.cursor.char_range())
            .map_or(len, |r| r.as_sorted_char_range().end.0.min(len));
        let text = item.placeholder();
        let needs_gap = at > 0 && !slice(&d.body, &(at - 1..at)).ends_with('\n');
        let insert = if needs_gap { format!("\n{text}\n") } else { format!("{text}\n") };
        replace(d, Field::Body, at..at, &insert);
        let after = at + insert.chars().count();
        set_cursor(ctx, Field::Body, after..after);
        d.images.push(item);
    }
}

fn image_strip(ui: &mut Ui, pal: &Palette, d: &mut Draft) {
    let mut remove = None;
    egui::ScrollArea::horizontal()
        .id_salt("feedback_images")
        .max_height(110.0)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                for (i, item) in d.images.iter_mut().enumerate() {
                    let texture = item.texture.get_or_insert_with(|| {
                        ui.ctx().load_texture(
                            format!("feedback_image_{}", item.number),
                            (*item.image).clone(),
                            Default::default(),
                        )
                    });
                    let [w, h] = item.image.size;
                    let scale = (80.0 / h.max(1) as f32).min(140.0 / w.max(1) as f32);
                    let size = egui::vec2(w as f32 * scale, h as f32 * scale);
                    ui.vertical(|ui| {
                        ui.add(egui::Image::new((texture.id(), size)).corner_radius(4));
                        ui.horizontal(|ui| {
                            widgets::note(ui, pal, trf("Image {0}", &[&item.number]));
                            if ui
                                .small_button("x")
                                .on_hover_text(tr("Remove this image"))
                                .clicked()
                            {
                                remove = Some(i);
                            }
                        });
                    });
                }
            });
        });
    if let Some(i) = remove {
        let item = d.images.remove(i);
        let placeholder = item.placeholder();
        if let Some(byte) = d.body.find(&placeholder) {
            let start = d.body[..byte].chars().count();
            let mut end = start + placeholder.chars().count();
            if slice(&d.body, &(end..end + 1)) == "\n" {
                end += 1;
            }
            replace(d, Field::Body, start..end, "");
        }
    }
}

/// The "Hyperlink" popup: Text, a divider, then Link.
fn hyperlink_dialog(ctx: &egui::Context, pal: &Palette, d: &mut Draft) {
    let Some(dialog) = &mut d.hyperlink else { return };
    let mut done: Option<bool> = None;
    let modal = egui::Modal::new(Id::unique("feedback_hyperlink")).show(ctx, |ui| {
        ui.set_width(360.0);
        ui.label(RichText::new(tr("Hyperlink")).family(semibold()).size(17.0));
        ui.add_space(8.0);
        let text = ui.add(
            egui::TextEdit::singleline(&mut dialog.text)
                .hint_text(tr("Text"))
                .desired_width(f32::INFINITY),
        );
        if !dialog.focused {
            if dialog.text.is_empty() {
                text.request_focus();
            }
            dialog.focused = true;
        }
        widgets::divider(ui);
        let url = ui.add(
            egui::TextEdit::singleline(&mut dialog.url)
                .hint_text(tr("Link"))
                .desired_width(f32::INFINITY),
        );
        if dialog.focused && !dialog.text.is_empty() && dialog.url.is_empty() && !text.has_focus() {
            url.request_focus();
        }
        let valid = normalize_url(&dialog.url).is_some() && !dialog.text.trim().is_empty();
        if !dialog.url.trim().is_empty() && normalize_url(&dialog.url).is_none() {
            widgets::note(ui, pal, tr("Links must be web addresses (http or https)."));
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if widgets::primary_button(ui, pal, tr("Add"), valid).clicked()
                || (valid && ui.input(|i| i.key_pressed(egui::Key::Enter)))
            {
                done = Some(true);
            }
            if widgets::button(ui, tr("Cancel"), true).clicked() {
                done = Some(false);
            }
        });
    });
    if modal.should_close() && done.is_none() {
        done = Some(false);
    }
    let Some(apply) = done else { return };
    let dialog = d.hyperlink.take().expect("dialog is open");
    if !apply {
        return;
    }
    let Some(url) = normalize_url(&dialog.url) else { return };
    let text = dialog.text.trim().to_string();
    let len = d.body.chars().count();
    let range = dialog.range.start.min(len)..dialog.range.end.min(len);
    replace(d, Field::Body, range.clone(), &text);
    let new = range.start..range.start + text.chars().count();
    d.links
        .retain(|l| l.range.end <= new.start || l.range.start >= new.end);
    d.links.push(Link {
        range: new.clone(),
        url,
    });
    set_cursor(ctx, Field::Body, new.end..new.end);
}

/// Only web links: anything else would be handed to Explorer as a path.
fn normalize_url(url: &str) -> Option<String> {
    let url = url.trim();
    if url.is_empty() || url.contains(char::is_whitespace) {
        return None;
    }
    let lower = url.to_ascii_lowercase();
    let full = if lower.starts_with("https://") || lower.starts_with("http://") {
        url.to_string()
    } else if lower.contains("://") || lower.starts_with("mailto:") || lower.contains('\\') {
        return None;
    } else {
        format!("https://{url}")
    };
    let host = full.split("://").nth(1)?.split(['/', '?', '#']).next()?;
    (host.contains('.') && !host.starts_with('.')).then_some(full)
}

fn open_link(log: &Logger, url: &str) {
    match normalize_url(url) {
        Some(url) => {
            if let Err(e) = shell::open_unelevated(&url) {
                log.error(e);
            }
        }
        None => log.warn(trf("Not opening {0}: it is not a web address.", &[&url])),
    }
}

// ---------------------------------------------------------------- sending

/// The body as Markdown: links become `[text](url)`, plus a short footer.
fn markdown(d: &Draft) -> String {
    let mut links: Vec<&Link> = d.links.iter().collect();
    links.sort_by_key(|l| l.range.start);
    let mut out = String::new();
    let mut at = 0;
    let len = d.body.chars().count();
    for link in links {
        let start = link.range.start.clamp(at, len);
        let end = link.range.end.clamp(start, len);
        out.push_str(&slice(&d.body, &(at..start)));
        let text = slice(&d.body, &(start..end)).replace('[', "\\[").replace(']', "\\]");
        out.push_str(&format!("[{text}]({})", link.url.replace(')', "%29")));
        at = end;
    }
    out.push_str(&slice(&d.body, &(at..len)));
    out = out.trim_end().to_string();
    out.push_str("\n\n---\n");
    if !d.images.is_empty() {
        out.push_str(&format!(
            "Screenshots: {} (drag them in from the folder RustCleanup opened)\n",
            d.images.len()
        ));
    }
    out.push_str(&format!("RustCleanup {} on {}\n", env!("CARGO_PKG_VERSION"), os_name()));
    out
}

#[cfg(windows)]
fn os_name() -> String {
    use crate::sys::registry::{self, Hive};
    const KEY: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
    let get = |name| registry::get_string(Hive::LocalMachine, KEY, name);
    let build: u32 = get("CurrentBuild").and_then(|b| b.parse().ok()).unwrap_or(0);
    // ProductName still says "Windows 10" on Windows 11.
    let name = if build >= 22000 { "Windows 11" } else { "Windows 10" };
    match get("DisplayVersion") {
        Some(v) => format!("{name} {v} (build {build})"),
        None => format!("{name} (build {build})"),
    }
}

/// The distro's name from `/etc/os-release` (falling back to
/// `/usr/lib/os-release`), plus the kernel version.
#[cfg(not(windows))]
fn os_name() -> String {
    let release = ["/etc/os-release", "/usr/lib/os-release"]
        .iter()
        .find_map(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_default();
    let name = os_release_name(&release).unwrap_or_else(|| "Linux".into());
    let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default();
    match kernel.trim() {
        "" => name,
        k => format!("{name} (kernel {k})"),
    }
}

/// `PRETTY_NAME`, or `NAME VERSION`, from an os-release file.
#[cfg(not(windows))]
fn os_release_name(text: &str) -> Option<String> {
    let field = |key: &str| {
        text.lines().find_map(|l| {
            let v = l.strip_prefix(key)?.strip_prefix('=')?.trim();
            let v = v.trim_matches(|c| c == '"' || c == '\'');
            (!v.is_empty()).then(|| v.to_string())
        })
    };
    field("PRETTY_NAME").or_else(|| match (field("NAME"), field("VERSION")) {
        (Some(n), Some(v)) => Some(format!("{n} {v}")),
        (n, _) => n,
    })
}

fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 3);
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// How to pick an email app on this system, for the Activity log.
fn mail_fix_hint() -> &'static str {
    if cfg!(windows) {
        tr("To fix this, pick an email app for MAILTO in Settings > Apps > Default apps, or send the report on GitHub instead.")
    } else {
        tr("To fix this, pick an email app in your desktop's default apps settings, or send the report on GitHub instead.")
    }
}

/// Looks up the app that opens email links before a draft is opened, and
/// logs anything that will stop the draft from appearing. Opening a link
/// cannot tell whether an app took it, so this is the only warning there is.
///
/// Returns the app's name when it is known, or the problem found.
fn check_mail(log: &Logger) -> (Option<String>, Option<MailProblem>) {
    use crate::sys::mail::MailHandler;
    match shell::mail_handler() {
        MailHandler::Missing => {
            let why = trf(
                "No email app is set up to open email links, so the email draft could not open. {0}",
                &[&mail_fix_hint()],
            );
            log.error(why.clone());
            (None, Some(MailProblem::Missing(why)))
        }
        MailHandler::App { name, browser: true } => {
            let why = trf(
                "Email links open in {0}, which is a web browser, so the email draft may not appear. {1}",
                &[&name, &mail_fix_hint()],
            );
            log.warn(why.clone());
            (Some(name), Some(MailProblem::Warning(why)))
        }
        MailHandler::App { name, browser: false } => (Some(name), None),
        MailHandler::Unknown(e) => {
            log.warn(trf("Could not check which app opens email links: {0}", &[&e]));
            (None, None)
        }
    }
}

/// Opens an email draft and logs where it went, or what went wrong.
fn open_mail(log: &Logger, url: &str, app: Option<&str>) -> bool {
    match shell::open_unelevated(url) {
        Ok(()) => {
            // Windows and xdg-open hand the link on and return at once, so
            // the only sign it went nowhere is the app never starting.
            if let Some(app) = app
                && shell::mail_app_running() == Some(false)
            {
                log.error(trf(
                    "{0} did not start, so no email draft opened. It may need to be set up or repaired first. {1}",
                    &[&app, &mail_fix_hint()],
                ));
                return false;
            }
            log.info(match app {
                Some(app) => trf("Email draft opened in {0}. Press Send there to finish.", &[&app]),
                None => tr("Asked your system to open an email draft. If none appears, check which app opens email links.").to_string(),
            });
            true
        }
        Err(e) => {
            log.error(trf("Could not open the email draft: {0}", &[&e]));
            false
        }
    }
}

fn send(ctx: &egui::Context, log: &Logger, d: &mut Draft, email: bool) {
    let (mail_app, mail_problem) = if email { check_mail(log) } else { (None, None) };
    if let Some(MailProblem::Missing(_)) = mail_problem {
        d.sent = Some(Sent {
            email,
            folder: None,
            on_clipboard: false,
            mail_problem,
        });
        return;
    }
    let title = format!("{} {}", d.kind.prefix(), d.title.trim());
    let body = markdown(d);
    let issue = |body: &str| {
        format!(
            "{NEW_ISSUE}?title={}&labels={}&body={}",
            encode(&title),
            d.kind.label_name(),
            encode(body)
        )
    };
    let mail = |body: &str| format!("mailto:{EMAIL}?subject={}&body={}", encode(&title), encode(body));
    let paste_here = tr("(The report is on your clipboard. Paste it here with Ctrl+V.)");

    let mut on_clipboard = false;
    let mut url = if email { mail(&body) } else { issue(&body) };
    if url.len() > if email { MAX_MAILTO } else { MAX_ISSUE_URL } {
        ctx.copy_text(body.clone());
        on_clipboard = true;
        url = if email { mail(paste_here) } else { issue(paste_here) };
    }
    let fallback = (!email).then(|| {
        let short = if mail(&body).len() > MAX_MAILTO {
            if !on_clipboard {
                ctx.copy_text(body.clone());
            }
            mail(paste_here)
        } else {
            mail(&body)
        };
        short
    });

    let images: Vec<(usize, Arc<ColorImage>)> =
        d.images.iter().map(|i| (i.number, i.image.clone())).collect();
    let folder = (!images.is_empty()).then(feedback_folder);
    let job_folder = folder.clone();
    let log = log.clone();
    std::thread::spawn(move || {
        if let Some(dir) = &job_folder {
            match save_images(dir, &images) {
                Ok(()) => {
                    if let Err(e) = shell::open_unelevated(&dir.to_string_lossy()) {
                        log.warn(e);
                    }
                }
                Err(e) => log.error(trf("Could not save the screenshots: {0}", &[&e])),
            }
        }
        if email {
            open_mail(&log, &url, mail_app.as_deref());
            return;
        }
        match shell::open_unelevated(&url) {
            Ok(()) => log.info(tr("Feedback opened on GitHub in your browser.")),
            Err(e) => {
                log.warn(trf("Could not open GitHub in your browser: {0}", &[&e]));
                // GitHub can't be reached through the browser: email instead.
                if let Some(mail) = fallback {
                    let (app, problem) = check_mail(&log);
                    if !matches!(problem, Some(MailProblem::Missing(_))) {
                        open_mail(&log, &mail, app.as_deref());
                    }
                }
            }
        }
    });
    d.sent = Some(Sent {
        email,
        folder,
        on_clipboard,
        mail_problem,
    });
}

fn feedback_folder() -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    std::env::temp_dir()
        .join("RustCleanup")
        .join("Feedback")
        .join(stamp.to_string())
}

fn save_images(dir: &std::path::Path, images: &[(usize, Arc<ColorImage>)]) -> Result<(), String> {
    // As root on Linux, the folders and files go to the desktop user, so a
    // later normal run can still write there and their apps can move them.
    #[cfg(not(windows))]
    crate::sys::user::create_dir(dir).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    for (number, image) in images {
        let path = dir.join(format!("{}.png", trf("Image {0}", &[number])));
        let file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
        let [w, h] = image.size;
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        let bytes: Vec<u8> = image
            .pixels
            .iter()
            .flat_map(|c| c.to_srgba_unmultiplied())
            .collect();
        writer.write_image_data(&bytes).map_err(|e| e.to_string())?;
        drop(writer);
        #[cfg(not(windows))]
        crate::sys::user::give_to_user(&path);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(range: Range<usize>) -> Vec<Link> {
        vec![Link {
            range,
            url: "https://example.com".into(),
        }]
    }

    #[test]
    fn links_follow_edits() {
        // "see docs here", link on "docs" (4..8)
        let base = "see docs here";
        let mut l = link(4..8);
        shift_links(&mut l, base, "please see docs here");
        assert_eq!(l[0].range, 11..15, "insert before moves it");

        let mut l = link(4..8);
        shift_links(&mut l, base, "see dXocs here");
        assert_eq!(l[0].range, 4..9, "insert inside grows it");

        let mut l = link(4..8);
        shift_links(&mut l, base, "see docs! here");
        assert_eq!(l[0].range, 4..8, "typing right after stays outside");

        let mut l = link(4..8);
        shift_links(&mut l, base, "see here");
        assert!(l.is_empty(), "deleting it drops it");

        let mut l = link(4..8);
        shift_links(&mut l, base, "see ds here");
        assert_eq!(l[0].range, 4..6, "deleting part shrinks it");
    }

    #[cfg(not(windows))]
    #[test]
    fn reads_os_release() {
        let text = "NAME=\"Ubuntu\"
VERSION=\"24.04 LTS\"
PRETTY_NAME=\"Ubuntu 24.04.1 LTS\"
";
        assert_eq!(os_release_name(text).as_deref(), Some("Ubuntu 24.04.1 LTS"));
        assert_eq!(os_release_name("NAME=Arch Linux
").as_deref(), Some("Arch Linux"));
        assert_eq!(os_release_name("NAME='Fedora'
VERSION=41
").as_deref(), Some("Fedora 41"));
    }

    #[test]
    fn only_web_links() {
        assert_eq!(normalize_url("example.com/a").as_deref(), Some("https://example.com/a"));
        assert_eq!(normalize_url("http://x.org").as_deref(), Some("http://x.org"));
        assert!(normalize_url(r"C:\Windows\System32\cmd.exe").is_none());
        assert!(normalize_url("file:///C:/x").is_none());
        assert!(normalize_url("notaurl").is_none());
    }

    #[test]
    fn markdown_links_and_encoding() {
        let d = Draft {
            body: "see docs here".into(),
            links: link(4..8),
            ..Default::default()
        };
        assert!(markdown(&d).starts_with("see [docs](https://example.com) here"));
        assert_eq!(encode("a b&c/ü"), "a%20b%26c%2F%C3%BC");
    }
}
