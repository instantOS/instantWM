use crate::Window;
use smithay::utils::{Rectangle, Size, Point};

pub trait Layout {
    fn arrange(&self, windows: &mut [Window], area: Rectangle<i32, smithay::utils::Physical>);
    fn symbol(&self) -> &str;
}

pub struct TileLayout {
    pub mfact: f32,
    pub nmaster: u32,
}

impl Default for TileLayout {
    fn default() -> Self {
        Self {
            mfact: 0.55,
            nmaster: 1,
        }
    }
}

impl Layout for TileLayout {
    fn symbol(&self) -> &str {
        "[]="
    }

    fn arrange(&self, windows: &mut [Window], area: Rectangle<i32, smithay::utils::Physical>) {
        let n = windows.len() as u32;
        if n == 0 {
            return;
        }

        if n > self.nmaster {
            let master_width = (area.size.w as f32 * self.mfact) as i32;
            let nmaster = self.nmaster as usize;
            let master_height = area.size.h / nmaster as i32;

            for (i, w) in windows.iter_mut().take(nmaster).enumerate() {
                w.geometry = Rectangle::from_loc_and_size(
                    (area.loc.x, area.loc.y + (i as i32 * master_height)),
                    (master_width, master_height),
                );
            }

            let stack_width = area.size.w - master_width;
            let nstack = n - self.nmaster;
            let stack_height = area.size.h / nstack as i32;

            for (i, w) in windows.iter_mut().skip(nmaster).enumerate() {
                w.geometry = Rectangle::from_loc_and_size(
                    (area.loc.x + master_width, area.loc.y + (i as i32 * stack_height)),
                    (stack_width, stack_height),
                );
            }
        } else {
            // only master windows
            let master_height = area.size.h / n as i32;
            for (i, w) in windows.iter_mut().enumerate() {
                w.geometry = Rectangle::from_loc_and_size(
                    (area.loc.x, area.loc.y + (i as i32 * master_height)),
                    (area.size.w, master_height),
                );
            }
        }
    }
}
