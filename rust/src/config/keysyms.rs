//! X11 keysym constants.
//!
//! These are the standard X11 keysym values used for key binding definitions.
//! Extracted here so they don't clutter the keybinding tables.

// Control / navigation
pub const XK_BackSpace: u32 = 0xFF08;
pub const XK_Tab: u32 = 0xFF09;
pub const XK_Return: u32 = 0xFF0D;
pub const XK_Escape: u32 = 0xFF1B;
pub const XK_Delete: u32 = 0xFFFF;
pub const XK_Home: u32 = 0xFF50;
pub const XK_Left: u32 = 0xFF51;
pub const XK_Up: u32 = 0xFF52;
pub const XK_Right: u32 = 0xFF53;
pub const XK_Down: u32 = 0xFF54;
pub const XK_Page_Up: u32 = 0xFF55;
pub const XK_Page_Down: u32 = 0xFF56;
pub const XK_End: u32 = 0xFF57;
pub const XK_Insert: u32 = 0xFF63;

// Function keys
pub const XK_F1: u32 = 0xFFBE;
pub const XK_F2: u32 = 0xFFBF;
pub const XK_F3: u32 = 0xFFC0;
pub const XK_F4: u32 = 0xFFC1;
pub const XK_F5: u32 = 0xFFC2;
pub const XK_F6: u32 = 0xFFC3;
pub const XK_F7: u32 = 0xFFC4;
pub const XK_F8: u32 = 0xFFC5;
pub const XK_F9: u32 = 0xFFC6;
pub const XK_F10: u32 = 0xFFC7;
pub const XK_F11: u32 = 0xFFC8;
pub const XK_F12: u32 = 0xFFC9;

// Misc printable
pub const XK_space: u32 = 0x0020;
pub const XK_exclam: u32 = 0x0021;
pub const XK_quotedbl: u32 = 0x0022;
pub const XK_numbersign: u32 = 0x0023;
pub const XK_dollar: u32 = 0x0024;
pub const XK_percent: u32 = 0x0025;
pub const XK_ampersand: u32 = 0x0026;
pub const XK_apostrophe: u32 = 0x0027;
pub const XK_parenleft: u32 = 0x0028;
pub const XK_parenright: u32 = 0x0029;
pub const XK_asterisk: u32 = 0x002A;
pub const XK_plus: u32 = 0x002B;
pub const XK_comma: u32 = 0x002C;
pub const XK_minus: u32 = 0x002D;
pub const XK_period: u32 = 0x002E;
pub const XK_slash: u32 = 0x002F;

// Digits
pub const XK_0: u32 = 0x0030;
pub const XK_1: u32 = 0x0031;
pub const XK_2: u32 = 0x0032;
pub const XK_3: u32 = 0x0033;
pub const XK_4: u32 = 0x0034;
pub const XK_5: u32 = 0x0035;
pub const XK_6: u32 = 0x0036;
pub const XK_7: u32 = 0x0037;
pub const XK_8: u32 = 0x0038;
pub const XK_9: u32 = 0x0039;

// Punctuation
pub const XK_colon: u32 = 0x003A;
pub const XK_semicolon: u32 = 0x003B;
pub const XK_less: u32 = 0x003C;
pub const XK_equal: u32 = 0x003D;
pub const XK_greater: u32 = 0x003E;
pub const XK_question: u32 = 0x003F;
pub const XK_at: u32 = 0x0040;

// Uppercase letters
pub const XK_A: u32 = 0x0041;
pub const XK_B: u32 = 0x0042;
pub const XK_C: u32 = 0x0043;
pub const XK_D: u32 = 0x0044;
pub const XK_E: u32 = 0x0045;
pub const XK_F: u32 = 0x0046;
pub const XK_G: u32 = 0x0047;
pub const XK_H: u32 = 0x0048;
pub const XK_I: u32 = 0x0049;
pub const XK_J: u32 = 0x004A;
pub const XK_K: u32 = 0x004B;
pub const XK_L: u32 = 0x004C;
pub const XK_M: u32 = 0x004D;
pub const XK_N: u32 = 0x004E;
pub const XK_O: u32 = 0x004F;
pub const XK_P: u32 = 0x0050;
pub const XK_Q: u32 = 0x0051;
pub const XK_R: u32 = 0x0052;
pub const XK_S: u32 = 0x0053;
pub const XK_T: u32 = 0x0054;
pub const XK_U: u32 = 0x0055;
pub const XK_V: u32 = 0x0056;
pub const XK_W: u32 = 0x0057;
pub const XK_X: u32 = 0x0058;
pub const XK_Y: u32 = 0x0059;
pub const XK_Z: u32 = 0x005A;

// Brackets / specials
pub const XK_bracketleft: u32 = 0x005B;
pub const XK_backslash: u32 = 0x005C;
pub const XK_bracketright: u32 = 0x005D;
pub const XK_asciicircum: u32 = 0x005E;
pub const XK_underscore: u32 = 0x005F;
pub const XK_grave: u32 = 0x0060;

// Lowercase letters
pub const XK_a: u32 = 0x0061;
pub const XK_b: u32 = 0x0062;
pub const XK_c: u32 = 0x0063;
pub const XK_d: u32 = 0x0064;
pub const XK_e: u32 = 0x0065;
pub const XK_f: u32 = 0x0066;
pub const XK_g: u32 = 0x0067;
pub const XK_h: u32 = 0x0068;
pub const XK_i: u32 = 0x0069;
pub const XK_j: u32 = 0x006A;
pub const XK_k: u32 = 0x006B;
pub const XK_l: u32 = 0x006C;
pub const XK_m: u32 = 0x006D;
pub const XK_n: u32 = 0x006E;
pub const XK_o: u32 = 0x006F;
pub const XK_p: u32 = 0x0070;
pub const XK_q: u32 = 0x0071;
pub const XK_r: u32 = 0x0072;
pub const XK_s: u32 = 0x0073;
pub const XK_t: u32 = 0x0074;
pub const XK_u: u32 = 0x0075;
pub const XK_v: u32 = 0x0076;
pub const XK_w: u32 = 0x0077;
pub const XK_x: u32 = 0x0078;
pub const XK_y: u32 = 0x0079;
pub const XK_z: u32 = 0x007A;

// Special
pub const XK_Print: u32 = 0xFF61;
pub const XK_dead_circumflex: u32 = 0xFE52;

// XF86 media / hardware keys
pub const XF86XK_MonBrightnessUp: u32 = 0x1008FF02;
pub const XF86XK_MonBrightnessDown: u32 = 0x1008FF03;
pub const XF86XK_AudioLowerVolume: u32 = 0x1008FF11;
pub const XF86XK_AudioMute: u32 = 0x1008FF12;
pub const XF86XK_AudioRaiseVolume: u32 = 0x1008FF13;
pub const XF86XK_AudioPlay: u32 = 0x1008FF14;
pub const XF86XK_AudioPause: u32 = 0x1008FF15;
pub const XF86XK_AudioNext: u32 = 0x1008FF17;
pub const XF86XK_AudioPrev: u32 = 0x1008FF16;
