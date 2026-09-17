use super::confirmed_stale_client;
use crate::model::WmModel;
use crate::types::{Client, WindowId};
use x11rb::errors::ReplyError;
use x11rb::protocol::ErrorKind;
use x11rb::x11_utils::X11Error;

const WINDOW: u32 = 71;

fn error(kind: ErrorKind, window: u32) -> X11Error {
    X11Error {
        error_kind: kind,
        error_code: 3,
        sequence: 1,
        bad_value: window,
        minor_opcode: 0,
        major_opcode: 3,
        extension_name: None,
        request_name: Some("GetWindowAttributes"),
    }
}

fn model() -> WmModel {
    let mut model = WmModel::default();
    assert!(model.insert_client(Client::new(WindowId::from(WINDOW))));
    model
}

#[test]
fn bad_window_requires_confirmation_for_the_managed_resource() {
    let model = model();
    let event = error(ErrorKind::Window, WINDOW);
    assert_eq!(
        confirmed_stale_client(&model, &event, |window| {
            assert_eq!(window, WINDOW);
            Err(ReplyError::X11Error(event.clone()))
        }),
        Some(WindowId::from(WINDOW))
    );
}

#[test]
fn live_or_reused_window_is_preserved() {
    assert_eq!(
        confirmed_stale_client(&model(), &error(ErrorKind::Window, WINDOW), |_| Ok(())),
        None
    );
}

#[test]
fn non_window_errors_do_not_even_query_attributes() {
    for kind in [ErrorKind::Drawable, ErrorKind::Match, ErrorKind::Access] {
        assert_eq!(
            confirmed_stale_client(&model(), &error(kind, WINDOW), |_| {
                panic!("only BadWindow can trigger a query")
            }),
            None
        );
    }
}

#[test]
fn unmanaged_resource_does_not_even_query_attributes() {
    assert_eq!(
        confirmed_stale_client(&model(), &error(ErrorKind::Window, WINDOW + 1), |_| {
            panic!("unmanaged resource must not trigger a query")
        }),
        None
    );
}

#[test]
fn other_reply_errors_are_not_proof_of_destruction() {
    let event = error(ErrorKind::Window, WINDOW);
    for kind in [ErrorKind::Drawable, ErrorKind::Match, ErrorKind::Access] {
        assert_eq!(
            confirmed_stale_client(&model(), &event, |_| {
                Err(ReplyError::X11Error(error(kind, WINDOW)))
            }),
            None
        );
    }
    assert_eq!(
        confirmed_stale_client(&model(), &event, |_| {
            Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe).into())
        }),
        None
    );
}

#[test]
fn bad_window_reply_for_another_resource_is_not_confirmation() {
    assert_eq!(
        confirmed_stale_client(&model(), &error(ErrorKind::Window, WINDOW), |_| {
            Err(ReplyError::X11Error(error(ErrorKind::Window, WINDOW + 1)))
        }),
        None
    );
}
