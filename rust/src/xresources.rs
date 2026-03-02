use crate::contexts::WmCtx;

const MAX_TAGLEN: usize = 16;

pub fn verify_tags_config(ctx: &mut WmCtx) {
    let mon_count = ctx.g.monitors.len();
    for mon_idx in 0..mon_count {
        for i in 0..9 {
            if let Some(mon) = ctx.g.monitors.get_mut(mon_idx) {
                if i < mon.tags.len() {
                    let len = mon.tags[i].name.len();
                    if len > MAX_TAGLEN - 1 || len == 0 {
                        mon.tags[i].name = "Config err".to_string();
                    }
                }
            }
        }
    }
}
