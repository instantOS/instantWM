use crate::contexts::WmCtx;

const MAX_TAGLEN: usize = 16;

pub fn verify_tags_config(ctx: &mut WmCtx) {
    for mon in ctx.g.monitors.iter_all_mut() {
        for i in 0..9 {
            if i < mon.tags.len() {
                let len = mon.tags[i].name.len();
                if len > MAX_TAGLEN - 1 || len == 0 {
                    mon.tags[i].name = "Config err".to_string();
                }
            }
        }
    }
}
