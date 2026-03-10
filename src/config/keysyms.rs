//! X11 keysym constants.
//!
//! These are the standard X11 keysym values used for key binding definitions.
//! Extracted here so they don't clutter the keybinding tables.
//!
//! Not every constant is referenced in the default bindings — they are provided
//! as a complete reference so custom bindings can use any key without needing
//! to look up the hex value.
#![allow(dead_code)]

// ---------------------------------------------------------------------------
// Control / navigation
// ---------------------------------------------------------------------------

pub const XK_BACKSPACE: u32 = 0xFF08;
pub const XK_TAB: u32 = 0xFF09;
pub const XK_RETURN: u32 = 0xFF0D;
pub const XK_ESCAPE: u32 = 0xFF1B;
pub const XK_DELETE: u32 = 0xFFFF;
pub const XK_HOME: u32 = 0xFF50;
pub const XK_LEFT: u32 = 0xFF51;
pub const XK_UP: u32 = 0xFF52;
pub const XK_RIGHT: u32 = 0xFF53;
pub const XK_DOWN: u32 = 0xFF54;
pub const XK_PAGE_UP: u32 = 0xFF55;
pub const XK_PAGE_DOWN: u32 = 0xFF56;
pub const XK_END: u32 = 0xFF57;
pub const XK_INSERT: u32 = 0xFF63;

// ---------------------------------------------------------------------------
// Function keys
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Whitespace / misc printable
// ---------------------------------------------------------------------------

pub const XK_SPACE: u32 = 0x0020;
pub const XK_EXCLAM: u32 = 0x0021;
pub const XK_QUOTE_DBL: u32 = 0x0022;
pub const XK_NUMBER_SIGN: u32 = 0x0023;
pub const XK_DOLLAR: u32 = 0x0024;
pub const XK_PERCENT: u32 = 0x0025;
pub const XK_AMPERSAND: u32 = 0x0026;
pub const XK_APOSTROPHE: u32 = 0x0027;
pub const XK_PAREN_LEFT: u32 = 0x0028;
pub const XK_PAREN_RIGHT: u32 = 0x0029;
pub const XK_ASTERISK: u32 = 0x002A;
pub const XK_PLUS: u32 = 0x002B;
pub const XK_COMMA: u32 = 0x002C;
pub const XK_MINUS: u32 = 0x002D;
pub const XK_PERIOD: u32 = 0x002E;
pub const XK_SLASH: u32 = 0x002F;

// ---------------------------------------------------------------------------
// Digits
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Punctuation
// ---------------------------------------------------------------------------

pub const XK_COLON: u32 = 0x003A;
pub const XK_SEMICOLON: u32 = 0x003B;
pub const XK_LESS: u32 = 0x003C;
pub const XK_EQUAL: u32 = 0x003D;
pub const XK_GREATER: u32 = 0x003E;
pub const XK_QUESTION: u32 = 0x003F;
pub const XK_AT: u32 = 0x0040;

// ---------------------------------------------------------------------------
// Uppercase letters  (Shift + key, or CapsLock)
// ---------------------------------------------------------------------------

pub const XK_A_UPPER: u32 = 0x0041;
pub const XK_B_UPPER: u32 = 0x0042;
pub const XK_C_UPPER: u32 = 0x0043;
pub const XK_D_UPPER: u32 = 0x0044;
pub const XK_E_UPPER: u32 = 0x0045;
pub const XK_F_UPPER: u32 = 0x0046;
pub const XK_G_UPPER: u32 = 0x0047;
pub const XK_H_UPPER: u32 = 0x0048;
pub const XK_I_UPPER: u32 = 0x0049;
pub const XK_J_UPPER: u32 = 0x004A;
pub const XK_K_UPPER: u32 = 0x004B;
pub const XK_L_UPPER: u32 = 0x004C;
pub const XK_M_UPPER: u32 = 0x004D;
pub const XK_N_UPPER: u32 = 0x004E;
pub const XK_O_UPPER: u32 = 0x004F;
pub const XK_P_UPPER: u32 = 0x0050;
pub const XK_Q_UPPER: u32 = 0x0051;
pub const XK_R_UPPER: u32 = 0x0052;
pub const XK_S_UPPER: u32 = 0x0053;
pub const XK_T_UPPER: u32 = 0x0054;
pub const XK_U_UPPER: u32 = 0x0055;
pub const XK_V_UPPER: u32 = 0x0056;
pub const XK_W_UPPER: u32 = 0x0057;
pub const XK_X_UPPER: u32 = 0x0058;
pub const XK_Y_UPPER: u32 = 0x0059;
pub const XK_Z_UPPER: u32 = 0x005A;

// ---------------------------------------------------------------------------
// Brackets / specials
// ---------------------------------------------------------------------------

pub const XK_BRACKET_LEFT: u32 = 0x005B;
pub const XK_BACKSLASH: u32 = 0x005C;
pub const XK_BRACKET_RIGHT: u32 = 0x005D;
pub const XK_ASCII_CIRCUM: u32 = 0x005E;
pub const XK_UNDERSCORE: u32 = 0x005F;
pub const XK_GRAVE: u32 = 0x0060;

// ---------------------------------------------------------------------------
// Lowercase letters  (unshifted letter keys)
// ---------------------------------------------------------------------------

pub const XK_A: u32 = 0x0061;
pub const XK_B: u32 = 0x0062;
pub const XK_C: u32 = 0x0063;
pub const XK_D: u32 = 0x0064;
pub const XK_E: u32 = 0x0065;
pub const XK_F: u32 = 0x0066;
pub const XK_G: u32 = 0x0067;
pub const XK_H: u32 = 0x0068;
pub const XK_I: u32 = 0x0069;
pub const XK_J: u32 = 0x006A;
pub const XK_K: u32 = 0x006B;
pub const XK_L: u32 = 0x006C;
pub const XK_M: u32 = 0x006D;
pub const XK_N: u32 = 0x006E;
pub const XK_O: u32 = 0x006F;
pub const XK_P: u32 = 0x0070;
pub const XK_Q: u32 = 0x0071;
pub const XK_R: u32 = 0x0072;
pub const XK_S: u32 = 0x0073;
pub const XK_T: u32 = 0x0074;
pub const XK_U: u32 = 0x0075;
pub const XK_V: u32 = 0x0076;
pub const XK_W: u32 = 0x0077;
pub const XK_X: u32 = 0x0078;
pub const XK_Y: u32 = 0x0079;
pub const XK_Z: u32 = 0x007A;

// ---------------------------------------------------------------------------
// Special keys
// ---------------------------------------------------------------------------

pub const XK_PRINT: u32 = 0xFF61;
/// Dead key: combining circumflex accent (^).
pub const XK_DEAD_CIRCUMFLEX: u32 = 0xFE52;

// ---------------------------------------------------------------------------
// XF86 media / hardware keys
// ---------------------------------------------------------------------------

pub const XF86XK_MON_BRIGHTNESS_UP: u32 = 0x1008FF02;
pub const XF86XK_MON_BRIGHTNESS_DOWN: u32 = 0x1008FF03;
pub const XF86XK_AUDIO_LOWER_VOLUME: u32 = 0x1008FF11;
pub const XF86XK_AUDIO_MUTE: u32 = 0x1008FF12;
pub const XF86XK_AUDIO_RAISE_VOLUME: u32 = 0x1008FF13;
pub const XF86XK_AUDIO_PLAY: u32 = 0x1008FF14;
pub const XF86XK_AUDIO_PAUSE: u32 = 0x1008FF15;
pub const XF86XK_AUDIO_NEXT: u32 = 0x1008FF17;
pub const XF86XK_AUDIO_PREV: u32 = 0x1008FF16;
