use crate::monitor::Monitor;
use crate::types::*;

// TODO: Port tiling layouts from layouts.c

pub fn tile(_m: &mut Monitor) {
    // TODO: Tile layout
}

pub fn monocle(_m: &mut Monitor) {
    // TODO: Monocle layout (single window fullscreen)
}

pub fn deck(_m: &mut Monitor) {
    // TODO: Deck layout
}

pub fn fibonacci(_m: &mut Monitor, _spiral: bool) {
    // TODO: Fibonacci/dwindle layout
}

pub fn spiral(_m: &mut Monitor) {
    fibonacci(m, true);
}

pub fn dwindle(_m: &mut Monitor) {
    fibonacci(m, false);
}

pub fn grid(_m: &mut Monitor) {
    // TODO: Grid layout
}

pub fn horizgrid(_m: &mut Monitor) {
    // TODO: Horizontal grid layout
}

pub fn gaplessgrid(_m: &mut Monitor) {
    // TODO: Gapless grid layout
}

pub fn bstack(_m: &mut Monitor) {
    // TODO: Bottom stack layout
}

pub fn set_layout(_arg: &Arg) {
    // TODO: Set current layout
}

pub fn cycle_layout(_arg: &Arg) {
    // TODO: Cycle through layouts
}

pub fn inc_nmaster(_arg: &Arg) {
    // TODO: Increment number of master windows
}

pub fn set_mfact(_arg: &Arg) {
    // TODO: Set master area factor
}
