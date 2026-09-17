use super::*;

// ── Text rendering ────────────────────────────────────────────────────────────

impl DrawContext {
    #[allow(clippy::too_many_arguments)]
    fn prepare_text_surface(
        &mut self,
        bounds: WmRect,
        lpad: u32,
        invert: bool,
        detail_height: i32,
        fg_pixel: u32,
        bg_pixel: u32,
        detail_pixel: u32,
    ) -> (bool, i32, u32, *mut XftDraw) {
        let mut x = bounds.x;
        let y = bounds.y;
        let mut w = bounds.w.max(0) as u32;
        let h = bounds.h.max(0) as u32;
        if self.display.is_null() {
            return (false, x, w, ptr::null_mut());
        }
        // Paint background and create Xft draw surface.
        // SAFETY: Xlib/Xft drawing calls with raw pointers.
        let bg = if invert { fg_pixel } else { bg_pixel };
        unsafe {
            XSetForeground(self.display, self.gc, bg as c_ulong);

            if detail_height > 0 {
                // Main background (above the detail strip).
                XFillRectangle(
                    self.display,
                    self.drawable,
                    self.gc,
                    x,
                    y,
                    w,
                    h.saturating_sub(detail_height as u32),
                );
                // Colored detail strip at the bottom.
                XSetForeground(self.display, self.gc, detail_pixel as c_ulong);
                XFillRectangle(
                    self.display,
                    self.drawable,
                    self.gc,
                    x,
                    y + h as i32 - detail_height,
                    w,
                    detail_height as u32,
                );
            } else {
                XFillRectangle(self.display, self.drawable, self.gc, x, y, w, h);
            }
        }

        let d = self.xft_draw;
        if d.is_null() {
            // Fallback to measure-only if the persistent Xft surface failed.
            return (false, x, u32::MAX, ptr::null_mut());
        }

        x += lpad as i32;
        w = w.saturating_sub(lpad);
        (true, x, w, d)
    }

    /// Render `text` into `bounds`.
    ///
    /// When `bounds` is empty at the origin, the function only measures
    /// the text and returns the advance width without drawing anything.
    ///
    /// # Parameters
    ///
    /// * `lpad`          — horizontal padding added before the first glyph.
    /// * `invert`        — swap fg/bg colors.
    /// * `detail_height` — if `> 0`, the bottom `detail_height` pixels of the
    ///   background are painted in the *detail* color.
    ///
    /// # Returns
    ///
    /// * In **render** mode (`w > 0`): `x + remaining_width` (the x position
    ///   just past the drawn area, suitable for chaining draw calls).
    /// * In **measure** mode (`w == 0`): total advance width of the text.
    pub fn text(
        &mut self,
        bounds: WmRect,
        lpad: u32,
        text: &str,
        invert: bool,
        detail_height: i32,
    ) -> i32 {
        if self.display.is_null() {
            return 0;
        }

        let cell_right = bounds.right();
        let mut x = bounds.x;
        let y = bounds.y;
        let mut w = bounds.w.max(0) as u32;
        let h = bounds.h.max(0) as u32;

        // ── Measure-only mode ────────────────────────────────────────────────
        // Measuring text width requires only the fontset, not a color scheme.
        let mut render = x != 0 || y != 0 || w != 0 || h != 0;
        if !render {
            if self.fonts.is_none() || text.is_empty() {
                return 0;
            }
            w = u32::MAX;
        }

        // ── Extract scheme colors (render path only) ─────────────────────────
        let mut fg_pixel: u32 = 0;
        let mut bg_pixel: u32 = 0;
        let mut detail_pixel: u32 = 0;
        let (fg_color, bg_color): (Option<XftColor>, Option<XftColor>) = if render {
            let Some(ref scheme) = self.scheme else {
                return 0;
            };
            fg_pixel = scheme.fg.pixel();
            bg_pixel = scheme.bg.pixel();
            detail_pixel = scheme.detail.pixel();
            (Some(scheme.fg.color.clone()), Some(scheme.bg.color.clone()))
        } else {
            (None, None)
        };

        // ── Prepare background + Xft draw surface ────────────────────────────
        let mut d: *mut XftDraw = ptr::null_mut();
        if render {
            let (r, nx, nw, nd) = self.prepare_text_surface(
                WmRect::new(x, y, w as i32, h as i32),
                lpad,
                invert,
                detail_height,
                fg_pixel,
                bg_pixel,
                detail_pixel,
            );
            render = r;
            x = nx;
            w = nw;
            d = nd;
        }

        // A text cell owns its background independently of whether it has a
        // drawable label. This also keeps layout stable when a configured tag
        // name is empty or a font failed to provide any glyphs.
        if text.is_empty() || self.fonts.is_none() {
            return if render { cell_right } else { 0 };
        }

        let (x, w) = self.text_run_loop(
            d,
            WmRect::new(x, y, 0, h as i32),
            w,
            text,
            invert,
            render,
            fg_color.as_ref(),
            bg_color.as_ref(),
        );

        x + if render { w as i32 } else { 0 }
    }

    /// Whether a font run starting after `prev` must lead with the inline-icon
    /// boundary gap before rendering `next`.
    ///
    /// Nerd-font icons carry zero side bearings, so a run starting next to an
    /// inline icon takes a leading gap; the icon's own side waits at the
    /// opposite transition via the same predicate. This single predicate
    /// governs both the leading gap charged when a run starts and the run
    /// split inside the walk (see `text_run_loop`), so the two can never
    /// drift apart.
    fn charges_boundary_gap(&self, prev: Option<char>, next: Option<char>) -> bool {
        self.icon_gap_px > 0
            && matches!(
                (prev, next),
                (Some(prev), Some(next))
                    if crate::bar::text::boundary_gap_between(prev, next)
            )
    }

    #[allow(clippy::too_many_arguments)]
    fn text_run_loop(
        &mut self,
        d: *mut XftDraw,
        bounds: WmRect,
        available_width: u32,
        text: &str,
        invert: bool,
        render: bool,
        fg_color: Option<&XftColor>,
        bg_color: Option<&XftColor>,
    ) -> (i32, u32) {
        let mut x = bounds.x;
        let y = bounds.y;
        // Keep the width unsigned all the way into the run loop. Measure-only
        // calls use `u32::MAX` as an unbounded width; storing that sentinel in
        // `Rect::w` used to wrap it to -1 and collapse every measurement to 0.
        let mut w = available_width;
        let h = bounds.h.max(0) as u32;
        if self.display.is_null() {
            return (x, w);
        }
        let text_bytes = text.as_bytes();
        let mut text_pos: usize = 0;

        let mut overflow = false;

        while text_pos < text_bytes.len() {
            let mut ew: u32 = 0;
            let mut utf8strlen: usize = 0;
            let utf8str_start = text_pos; // byte offset of this font-run's start
            let first = text[text_pos..].chars().next().unwrap();
            let usedfont_idx = self.font_for_char(first);

            // Pen and remaining width move together so overflow checks below
            // see numbers that already include the pad.
            if self.charges_boundary_gap(
                text[..utf8str_start].chars().next_back(),
                text[utf8str_start..].chars().next(),
            ) {
                let gap = self.icon_gap_px.min(w);
                x += gap as i32;
                w -= gap;
            }

            // ── Walk codepoints in the current font run ──────────────────────
            while text_pos < text_bytes.len() {
                // `text_pos` is advanced only by UTF-8 scalar lengths, and
                // the input is already a valid `&str`; validating the entire
                // remaining suffix for every character made this loop
                // quadratic for long titles.
                let ch = text[text_pos..]
                    .chars()
                    .next()
                    .expect("text_pos must precede a UTF-8 scalar");
                let charlen = ch.len_utf8();
                if utf8strlen > 0 {
                    // A semantic boundary needs its own run even if both roles
                    // ended up using the same face for .notdef.
                    if self.charges_boundary_gap(
                        text[..text_pos].chars().next_back(),
                        Some(ch),
                    ) || self.font_for_char(ch) != usedfont_idx
                    {
                        break;
                    }
                }
                let font = &self.fonts.as_ref().unwrap()[usedfont_idx];
                let tmpw = self.font_getexts(font, &text_bytes[text_pos..text_pos + charlen]);
                if tmpw > w.saturating_sub(ew) {
                    overflow = true;
                    if !render {
                        x += tmpw as i32;
                    }
                    break;
                }

                // Font resolution is complete, including failed fallback:
                // Xft measures/draws glyph zero (.notdef) for missing scalars.
                // Every non-overflowing iteration consumes one whole scalar.
                utf8strlen += charlen;
                text_pos += charlen;
                ew += tmpw;
            }

            // ── Render the accumulated run ───────────────────────────────────
            if utf8strlen > 0 {
                if render {
                    let fg_color =
                        fg_color.expect("text_run_loop: fg_color required in render mode");
                    let bg_color =
                        bg_color.expect("text_run_loop: bg_color required in render mode");
                    let f = self
                        .fonts
                        .as_ref()
                        .expect("font cache must be initialized before drawing")
                        .get(usedfont_idx)
                        .expect("usedfont_idx exceeds font cache length");
                    let ty = y + ((h as i32 - f.h as i32) / 2) + f.ascent();

                    let run_bytes = &text_bytes[utf8str_start..utf8str_start + utf8strlen];
                    unsafe {
                        XftDrawStringUtf8(
                            d,
                            if invert { bg_color } else { fg_color } as *const XftColor,
                            f.xfont,
                            x as c_int,
                            ty as c_int,
                            run_bytes.as_ptr(),
                            utf8strlen as c_int,
                        );
                    }
                }

                x += ew as i32;
                w = w.saturating_sub(ew);
            }

            if text_pos >= text_bytes.len() || overflow {
                break;
            }
        }

        (x, w)
    }

    /// Resolve a scalar before advancing the run. A failed lookup is a final
    /// choice too: use the role's base face and let Xft render its .notdef.
    fn font_for_char(&mut self, ch: char) -> usize {
        let wanted_role = crate::bar::text::role_for_char(ch);
        let fonts = self.fonts.as_ref().expect("font cache must be initialized");
        if let Some(idx) = fonts.iter().position(|font| {
            font.role == wanted_role
                && unsafe { XftCharExists(self.display, font.xfont, ch as u32) != 0 }
        }) {
            return idx;
        }
        let base_idx = fonts
            .iter()
            .position(|font| font.role == wanted_role)
            .unwrap_or(0);
        if !self.is_nomatch(ch as u32)
            && let Some(idx) = self.try_load_fallback_font(ch as u32)
        {
            return idx;
        }
        base_idx
    }

    /// Return `true` if `codepoint` is in the no-match cache.
    fn is_nomatch(&self, codepoint: u32) -> bool {
        self.nomatches.contains(&codepoint)
    }

    /// Attempt to find a Fontconfig fallback font that contains `codepoint`.
    ///
    /// On success append the new font and return its index. On failure record
    /// the codepoint in the no-match cache; the caller chooses the .notdef face.
    fn try_load_fallback_font(&mut self, codepoint: u32) -> Option<usize> {
        if self.display.is_null() {
            return None;
        }
        unsafe {
            let fccharset = FcCharSetCreate();
            FcCharSetAddChar(fccharset, codepoint);

            let fonts_ref = self
                .fonts
                .as_ref()
                .expect("font cache must be initialized before fallback lookup");
            let wanted_role = crate::bar::text::role_for_char(
                char::from_u32(codepoint).unwrap_or(char::REPLACEMENT_CHARACTER),
            );
            let base_font = fonts_ref
                .iter()
                .find(|font| font.role == wanted_role)
                .unwrap_or(&fonts_ref[0]);
            if base_font.pattern.is_null() {
                panic!("draw: fallback base font must be loaded from a font name string.");
            }

            let fcpattern = FcPatternDuplicate(base_font.pattern);
            FcPatternAddCharSet(fcpattern, FC_CHARSET.as_ptr(), fccharset);
            FcPatternAddBool(fcpattern, FC_SCALABLE.as_ptr(), FC_TRUE);
            FcConfigSubstitute(ptr::null_mut(), fcpattern, FC_MATCH_PATTERN);
            FcDefaultSubstitute(fcpattern);

            let mut result: XftResult = 0;
            let match_pattern = XftFontMatch(self.display, self.screen, fcpattern, &mut result);

            FcCharSetDestroy(fccharset);
            FcPatternDestroy(fcpattern);

            if !match_pattern.is_null()
                && let Ok(Some(new_font)) =
                    self.xfont_create(wanted_role, None, Some(match_pattern))
                && XftCharExists(self.display, new_font.xfont, codepoint) != 0
            {
                let fonts = self.fonts.as_mut().unwrap();
                let idx = fonts.len();
                fonts.push(new_font);
                return Some(idx);
            }
        }
        if self.nomatches.len() >= NOMATCHES_LEN {
            self.nomatches.pop_front();
        }
        self.nomatches.push_back(codepoint);
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bar::text::FontRole;

    // Run on a dedicated server, not a user's desktop:
    // xvfb-run -a cargo test backend::x11::draw::context::text::tests -- --ignored --test-threads=1
    fn reset_fonts(ctx: &mut DrawContext) {
        ctx.nomatches.clear();
        ctx.fontset_create(&[
            (FontRole::Text, "DejaVu Sans Mono:pixelsize=14"),
            (FontRole::Icon, "DejaVu Sans Mono:pixelsize=28"),
        ])
        .unwrap();
        ctx.resize(1024, 64);
        let scheme = ctx.scm_create(&["#ffffff", "#000000", "#000000"]).unwrap();
        ctx.set_scheme(scheme);
        ctx.set_icon_gap_px(5);
    }

    #[test]
    #[ignore = "requires a dedicated Xvfb display and DejaVu fonts"]
    fn xft_glyph_progress_regressions() {
        // Use one display for the suite, as the compositor does.
        let mut ctx = DrawContext::new(None).expect("tests require Xvfb and DejaVu fonts");
        reset_fonts(&mut ctx);
        missing_glyphs_consume_scalars_and_use_role_notdef(&mut ctx);
        reset_fonts(&mut ctx);
        dynamic_fallback_is_consumed_and_reused(&mut ctx);
        reset_fonts(&mut ctx);
        missing_icon_spacing_and_overflow_are_applied_once(&mut ctx);
        unsafe extern "C" {
            fn XSync(display: *mut libc::c_void, discard: c_int) -> c_int;
        }
        unsafe { XSync(ctx.display, 0) };
    }

    fn glyph_width(ctx: &DrawContext, idx: usize, text: &str) -> u32 {
        ctx.font_getexts(&ctx.fonts.as_ref().unwrap()[idx], text.as_bytes())
    }

    fn assert_measure_and_render(ctx: &mut DrawContext, text: &str, expected: u32) {
        // Call text directly so repeated cases exercise the no-match/font
        // caches rather than returning from the whole-string width cache.
        assert_eq!(
            ctx.text(WmRect::default(), 0, text, false, 0),
            expected as i32
        );
        let scheme = ctx.get_scheme().unwrap().clone();
        for invert in [false, true] {
            let (x, remaining) = ctx.text_run_loop(
                ctx.xft_draw,
                WmRect::new(7, 0, 0, 64),
                1000,
                text,
                invert,
                true,
                Some(&scheme.fg.color),
                Some(&scheme.bg.color),
            );
            assert_eq!(x, 7 + expected as i32);
            assert_eq!(remaining, 1000 - expected);
        }
    }

    fn missing_glyphs_consume_scalars_and_use_role_notdef(ctx: &mut DrawContext) {
        for (missing, idx) in [("\u{10ffff}", 0), ("\u{10fffd}", 1)] {
            let codepoint = missing.chars().next().unwrap() as u32;
            assert_eq!(
                unsafe {
                    XftCharExists(
                        ctx.display,
                        ctx.fonts.as_ref().unwrap()[idx].xfont,
                        codepoint,
                    )
                },
                0
            );
            let notdef = glyph_width(ctx, idx, missing);
            assert!(notdef > 0);
            let normal = glyph_width(ctx, 0, "AB");
            let gap = if idx == 1 { 10 } else { 0 };
            let text = format!("A{missing}{missing}B");
            assert!(!ctx.is_nomatch(codepoint));
            assert_measure_and_render(ctx, &text, normal + notdef * 2 + gap);
            assert!(ctx.is_nomatch(codepoint));
            assert_eq!(ctx.fonts.as_ref().unwrap().len(), 2);
            assert_eq!(ctx.font_for_char(missing.chars().next().unwrap()), idx);
            assert_measure_and_render(ctx, missing, notdef);
            assert_measure_and_render(
                ctx,
                &format!("{missing}AB{missing}"),
                normal + notdef * 2 + gap,
            );
            assert_eq!(
                ctx.nomatches.iter().filter(|&&cp| cp == codepoint).count(),
                1
            );
        }
    }

    fn dynamic_fallback_is_consumed_and_reused(ctx: &mut DrawContext) {
        let glyph = "\u{1f600}";
        assert_eq!(
            unsafe { XftCharExists(ctx.display, ctx.fonts.as_ref().unwrap()[0].xfont, 0x1f600) },
            0
        );
        // DejaVu Sans has this glyph; DejaVu Sans Mono does not.
        let width = ctx.text(WmRect::default(), 0, glyph, false, 0);
        assert!(width > 0);
        assert_eq!(ctx.fonts.as_ref().unwrap().len(), 3);
        assert_eq!(ctx.fonts.as_ref().unwrap()[2].role, FontRole::Text);
        assert!(!ctx.is_nomatch(0x1f600));
        assert_eq!(ctx.font_for_char('\u{1f600}'), 2);
        assert_eq!(width as u32, glyph_width(ctx, 2, glyph));
        let normal = glyph_width(ctx, 0, "AB");
        assert_measure_and_render(ctx, "A😀😀B", normal + width as u32 * 2);
        assert_eq!(ctx.fonts.as_ref().unwrap().len(), 3);
    }

    fn missing_icon_spacing_and_overflow_are_applied_once(ctx: &mut DrawContext) {
        // Also cover an absent role: .notdef shares the text face, but
        // semantic gaps must still split the run and appear exactly once.
        ctx.fonts.as_mut().unwrap().truncate(1);
        let icon = "\u{10fffd}";
        let notdef = glyph_width(ctx, 0, icon);
        let normal = glyph_width(ctx, 0, "AB");
        assert_measure_and_render(ctx, &format!("A{icon}B"), normal + notdef + 10);
        let scheme = ctx.get_scheme().unwrap().clone();
        let a = glyph_width(ctx, 0, "A");
        for width in [0, a, a + 3, a + 5, a + 5 + notdef - 1, a + 5 + notdef] {
            let (x, remaining) = ctx.text_run_loop(
                ctx.xft_draw,
                WmRect::new(0, 0, 0, 64),
                width,
                &format!("A{icon}B"),
                false,
                true,
                Some(&scheme.fg.color),
                Some(&scheme.bg.color),
            );
            let expected = if width < a {
                0
            } else if width < a + 5 + notdef {
                a + 5.min(width - a)
            } else {
                width
            };
            assert_eq!(x, expected as i32, "width={width}");
            assert_eq!(remaining, width - expected);
        }
        assert_measure_and_render(ctx, "", 0);
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

pub(super) fn zero_glyph_info() -> XGlyphInfo {
    XGlyphInfo {
        width: 0,
        height: 0,
        x: 0,
        y: 0,
        x_off: 0,
        y_off: 0,
    }
}
