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

        let mut usedfont_idx: usize = 0;
        let mut charexists = false;

        let mut overflow = false;

        loop {
            let mut ew: u32 = 0;
            let mut utf8strlen: usize = 0;
            let utf8str_start = text_pos; // byte offset of this font-run's start
            let mut nextfont_idx: Option<usize> = None;
            let mut utf8codepoint: u32 = 0;

            // Nerd-font icons carry zero side bearings, so a run starting next
            // to an inline icon takes a leading gap; the icon's own side waits
            // at the opposite transition via the same predicate. Pen and
            // remaining width move together so overflow checks below see
            // numbers that already include the pad.
            if self.icon_gap_px > 0
                && let (Some(prev), Some(first)) = (
                    text[..utf8str_start].chars().next_back(),
                    text[utf8str_start..].chars().next(),
                )
                && crate::bar::text::boundary_gap_between(prev, first)
            {
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
                let (charlen, codepoint) = (ch.len_utf8(), ch as u32);
                utf8codepoint = codepoint;

                // Find which font in the fallback chain can render this char.
                while !charexists {
                    let font_count = self
                        .fonts
                        .as_ref()
                        .expect("font cache must be initialized before drawing")
                        .len();
                    let wanted_role = crate::bar::text::role_for_char(
                        char::from_u32(utf8codepoint).unwrap_or(char::REPLACEMENT_CHARACTER),
                    );
                    // Only consult the face assigned to this semantic role.
                    // Missing glyphs proceed to Fontconfig fallback using that
                    // role's pattern and therefore retain the intended size.
                    for cur_idx in 0..font_count {
                        let cur_font = self
                            .fonts
                            .as_ref()
                            .expect("font cache must be initialized before drawing")
                            .get(cur_idx)
                            .expect("font index out of bounds within font cache");
                        if cur_font.role != wanted_role {
                            continue;
                        }

                        charexists = unsafe {
                            XftCharExists(self.display, cur_font.xfont, utf8codepoint) != 0
                        };
                        if !charexists {
                            continue;
                        }
                        let slice_end = (text_pos + charlen).min(text_bytes.len());
                        let glyph_bytes = &text_bytes[text_pos..slice_end];
                        let tmpw = self.font_getexts(cur_font, glyph_bytes);

                        if ew + tmpw > w {
                            // This glyph would overflow — stop the run here.
                            overflow = true;
                            if !render {
                                x += tmpw as i32;
                            }
                        } else if cur_idx == usedfont_idx {
                            // Same font — extend the current run.
                            utf8strlen += charlen;
                            text_pos += charlen;
                            ew += tmpw;
                        } else {
                            // Different font — end the current run; the
                            // outer loop will start a new one.
                            nextfont_idx = Some(cur_idx);
                        }
                        break;
                    }

                    if !charexists {
                        break;
                    }
                }

                if overflow || !charexists || nextfont_idx.is_some() {
                    break;
                }
                charexists = false;
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

            // ── Advance to the next font run ─────────────────────────────────
            if let Some(next_idx) = nextfont_idx {
                charexists = false;
                usedfont_idx = next_idx;
            } else {
                // No font in the set could render the codepoint — try dynamic
                // fallback via Fontconfig.
                charexists = true;

                if !self.is_nomatch(utf8codepoint) {
                    self.try_load_fallback_font(utf8codepoint, &mut usedfont_idx);
                } else {
                    usedfont_idx = 0;
                }
            }
        }

        (x, w)
    }

    /// Return `true` if `codepoint` is in the no-match cache.
    fn is_nomatch(&self, codepoint: u32) -> bool {
        self.nomatches.contains(&codepoint)
    }

    /// Attempt to find a Fontconfig fallback font that contains `codepoint`.
    ///
    /// On success the new font is appended to `self.fonts` and `usedfont_idx`
    /// is updated to point at it.  On failure the codepoint is recorded in the
    /// no-match cache and `usedfont_idx` is reset to 0.
    fn try_load_fallback_font(&mut self, codepoint: u32, usedfont_idx: &mut usize) {
        if self.display.is_null() {
            return;
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

            if match_pattern.is_null() {
                // No fallback found.
                if self.nomatches.len() >= NOMATCHES_LEN {
                    self.nomatches.pop_front();
                }
                self.nomatches.push_back(codepoint);
                *usedfont_idx = 0;
                return;
            }

            match self.xfont_create(wanted_role, None, Some(match_pattern)) {
                Ok(Some(new_font)) => {
                    if XftCharExists(self.display, new_font.xfont, codepoint) != 0 {
                        *usedfont_idx = self
                            .fonts
                            .as_ref()
                            .expect("font cache must be initialized before fallback lookup")
                            .len();
                        self.fonts
                            .as_mut()
                            .expect("font cache must be initialized before fallback lookup")
                            .push(new_font);
                    } else {
                        drop(new_font);
                        if self.nomatches.len() >= NOMATCHES_LEN {
                            self.nomatches.pop_front();
                        }
                        self.nomatches.push_back(codepoint);
                        *usedfont_idx = 0;
                    }
                }
                _ => {
                    if self.nomatches.len() >= NOMATCHES_LEN {
                        self.nomatches.pop_front();
                    }
                    self.nomatches.push_back(codepoint);
                    *usedfont_idx = 0;
                }
            }
        }
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
