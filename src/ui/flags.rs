//! Small flags for the language picker, painted with shapes.
//!
//! Windows has no colour flag emoji (Segoe UI Emoji shows two letters
//! instead), so each flag is drawn here. They are simplified for a 21x14
//! point size, but keep each flag's real colours and layout.

use eframe::egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, Ui, Vec2, pos2, vec2};

use crate::i18n::Lang;

pub const SIZE: Vec2 = vec2(21.0, 14.0);

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

const WHITE: Color32 = rgb(0xFFFFFF);

/// Allocates space and paints the flag for `lang`.
pub fn flag(ui: &mut Ui, lang: Lang) {
    let (rect, _) = ui.allocate_exact_size(SIZE, eframe::egui::Sense::hover());
    paint(ui.painter(), rect, lang);
}

/// Equal horizontal stripes, top to bottom.
fn horizontal(p: &Painter, r: Rect, colors: &[Color32]) {
    let h = r.height() / colors.len() as f32;
    for (i, c) in colors.iter().enumerate() {
        let top = r.top() + h * i as f32;
        p.rect_filled(
            Rect::from_min_max(pos2(r.left(), top), pos2(r.right(), top + h)),
            0.0,
            *c,
        );
    }
}

/// Equal vertical stripes, left to right.
fn vertical(p: &Painter, r: Rect, colors: &[Color32]) {
    let w = r.width() / colors.len() as f32;
    for (i, c) in colors.iter().enumerate() {
        let left = r.left() + w * i as f32;
        p.rect_filled(
            Rect::from_min_max(pos2(left, r.top()), pos2(left + w, r.bottom())),
            0.0,
            *c,
        );
    }
}

/// A five-pointed star, built from convex pieces (egui fills convex shapes only).
fn star(p: &Painter, center: Pos2, outer: f32, rotation: f32, color: Color32) {
    let inner = outer * 0.382;
    let point = |i: usize, radius: f32| {
        let a = rotation - std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
        center + vec2(a.cos(), a.sin()) * radius
    };
    let pentagon: Vec<Pos2> = (0..5).map(|i| point(2 * i + 1, inner)).collect();
    p.add(Shape::convex_polygon(pentagon, color, Stroke::NONE));
    for i in 0..5 {
        p.add(Shape::convex_polygon(
            vec![
                point(2 * i, outer),
                point(2 * i + 1, inner),
                point((2 * i + 9) % 10, inner),
            ],
            color,
            Stroke::NONE,
        ));
    }
}

pub fn paint(p: &Painter, r: Rect, lang: Lang) {
    let p = &p.with_clip_rect(r);
    match lang {
        Lang::En => union_jack(p, r),
        Lang::Es => {
            // Red, yellow (twice as tall), red.
            p.rect_filled(r, 0.0, rgb(0xAA151B));
            let band = r.shrink2(vec2(0.0, r.height() / 4.0));
            p.rect_filled(band, 0.0, rgb(0xF1BF00));
        }
        Lang::Pt => {
            let split = r.left() + r.width() * 0.4;
            p.rect_filled(
                Rect::from_min_max(r.min, pos2(split, r.bottom())),
                0.0,
                rgb(0x006600),
            );
            p.rect_filled(
                Rect::from_min_max(pos2(split, r.top()), r.max),
                0.0,
                rgb(0xFF0000),
            );
            let c = pos2(split, r.center().y);
            p.circle_filled(c, r.height() * 0.3, rgb(0xFFCC00));
            p.circle_filled(c, r.height() * 0.17, rgb(0xFF0000));
            p.circle_filled(c, r.height() * 0.11, WHITE);
        }
        Lang::Fr => vertical(p, r, &[rgb(0x0055A4), WHITE, rgb(0xEF4135)]),
        Lang::De => horizontal(p, r, &[rgb(0x000000), rgb(0xDD0000), rgb(0xFFCE00)]),
        Lang::It => vertical(p, r, &[rgb(0x009246), WHITE, rgb(0xCE2B37)]),
        Lang::Nl => horizontal(p, r, &[rgb(0xAE1C28), WHITE, rgb(0x21468B)]),
        Lang::Pl => horizontal(p, r, &[WHITE, rgb(0xDC143C)]),
        Lang::Tr => {
            let red = rgb(0xE30A17);
            p.rect_filled(r, 0.0, red);
            let h = r.height();
            let c = pos2(r.left() + r.width() * 0.36, r.center().y);
            p.circle_filled(c, h * 0.25, WHITE);
            p.circle_filled(c + vec2(h * 0.0625, 0.0), h * 0.2, red);
            star(
                p,
                c + vec2(h * 0.33, 0.0),
                h * 0.125,
                -std::f32::consts::FRAC_PI_2,
                WHITE,
            );
        }
        Lang::Ru => horizontal(p, r, &[WHITE, rgb(0x0039A6), rgb(0xD52B1E)]),
        Lang::Ja => {
            p.rect_filled(r, 0.0, WHITE);
            p.circle_filled(r.center(), r.height() * 0.3, rgb(0xBC002D));
        }
        Lang::Zh => {
            let yellow = rgb(0xFFDE00);
            p.rect_filled(r, 0.0, rgb(0xEE1C25));
            let unit = r.height() / 20.0;
            let at = |x: f32, y: f32| pos2(r.left() + x * unit, r.top() + y * unit);
            star(p, at(5.0, 5.0), 3.0 * unit, 0.0, yellow);
            for (x, y) in [(10.0, 2.0), (12.0, 4.0), (12.0, 7.0), (10.0, 9.0)] {
                // The small stars point at the big one.
                let to_big = (at(5.0, 5.0) - at(x, y)).angle() + std::f32::consts::FRAC_PI_2;
                star(p, at(x, y), unit, to_big, yellow);
            }
        }
    }
    // A hairline edge, so white flags don't melt into a light background.
    p.rect_stroke(
        r,
        1.0,
        Stroke::new(1.0, Color32::from_black_alpha(60)),
        eframe::egui::StrokeKind::Inside,
    );
}

/// The Union Jack, as a simplified version that reads at small sizes.
fn union_jack(p: &Painter, r: Rect) {
    let blue = rgb(0x012169);
    let red = rgb(0xC8102E);
    p.rect_filled(r, 0.0, blue);
    let h = r.height();
    let diagonals = [
        [r.left_top(), r.right_bottom()],
        [r.right_top(), r.left_bottom()],
    ];
    for d in diagonals {
        p.line_segment(d, Stroke::new(h * 0.2, WHITE));
    }
    for d in diagonals {
        p.line_segment(d, Stroke::new(h * 0.07, red));
    }
    let c = r.center();
    p.rect_filled(
        Rect::from_center_size(c, vec2(r.width(), h / 3.0)),
        0.0,
        WHITE,
    );
    p.rect_filled(Rect::from_center_size(c, vec2(h / 3.0, h)), 0.0, WHITE);
    p.rect_filled(
        Rect::from_center_size(c, vec2(r.width(), h / 5.0)),
        0.0,
        red,
    );
    p.rect_filled(Rect::from_center_size(c, vec2(h / 5.0, h)), 0.0, red);
}
