use crate::types::*;

pub fn tile(_m: &mut MonitorInner) {}

pub fn monocle(_m: &mut MonitorInner) {}

pub fn deck(_m: &mut MonitorInner) {}

pub fn fibonacci(_m: &mut MonitorInner, _spiral: bool) {}

pub fn spiral(m: &mut MonitorInner) {
    fibonacci(m, true);
}

pub fn dwindle(m: &mut MonitorInner) {
    fibonacci(m, false);
}

pub fn grid(_m: &mut MonitorInner) {}

pub fn horizgrid(_m: &mut MonitorInner) {}

pub fn gaplessgrid(_m: &mut MonitorInner) {}

pub fn bstack(_m: &mut MonitorInner) {}

pub fn tcl(_m: &mut MonitorInner) {}

pub fn overviewlayout(_m: &mut MonitorInner) {}

pub fn bstackhoriz(_m: &mut MonitorInner) {}

pub fn set_layout(_arg: &Arg) {}

pub fn cycle_layout(_arg: &Arg) {}

pub fn inc_nmaster(_arg: &Arg) {}

pub fn set_mfact(_arg: &Arg) {}
