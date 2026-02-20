use crate::globals::*;

// TODO: Port X resources loading from xresources.c

pub fn load_xresources() {
    // TODO: Load X resources
}

pub fn resource_load(
    _db: *mut std::ffi::c_void,
    _name: &str,
    _rtype: crate::types::ResourceType,
    _dst: *mut std::ffi::c_void,
) {
    // TODO: Load individual resource
}

pub fn verify_tags_xres() {
    // TODO: Verify tags from X resources
}
