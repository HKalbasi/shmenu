use std::env::set_current_dir;
use std::error::Error;

use x11rb::connection::{Connection, RequestConnection as _};
use x11rb::errors::ReplyOrIdError;
use x11rb::protocol::xkb::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{
    self, ConnectionExt as _, CreateWindowAux, EventMask, PropMode, WindowClass,
};
use x11rb::protocol::Event;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::xcb_ffi::XCBConnection;
use x11rb::{atom_manager, CURRENT_TIME};
use x11rb_protocol::protocol::xproto::*;
use xkbcommon::xkb as xkbc;

use crate::history_completer::{add_history_record, get_history_candidate};

mod history_completer;

// A collection of the atoms we will need.
atom_manager! {
    pub AtomCollection: AtomCollectionCookie {
        WM_PROTOCOLS,
        WM_DELETE_WINDOW,
        _NET_WM_NAME,
        UTF8_STRING,
    }
}

/// Handle a single key press or key release event
fn handle_key(event: xproto::KeyPressEvent, state: &xkbc::State) -> (u32, String) {
    let sym = state.key_get_one_sym(event.detail.into());
    let utf8 = state.key_get_utf8(event.detail.into());
    (sym, utf8)
}

/// Create and return a window
fn create_window<'a>(
    conn: &XCBConnection,
    screen_num: usize,
    atoms: &AtomCollection,
) -> Result<xproto::Window, ReplyOrIdError> {
    let screen = &conn.setup().roots[screen_num];
    let window = conn.generate_id()?;
    conn.create_window(
        screen.root_depth,
        window,
        screen.root,
        0,
        0,
        screen.width_in_pixels,
        30,
        0,
        WindowClass::INPUT_OUTPUT,
        screen.root_visual,
        &CreateWindowAux::new()
            .background_pixel(screen.black_pixel)
            .override_redirect(1)
            .event_mask(EventMask::KEY_PRESS | EventMask::KEY_RELEASE),
    )?;
    conn.change_property32(
        PropMode::REPLACE,
        window,
        atoms.WM_PROTOCOLS,
        xproto::AtomEnum::ATOM,
        &[atoms.WM_DELETE_WINDOW],
    )?;
    conn.change_property8(
        PropMode::REPLACE,
        window,
        xproto::AtomEnum::WM_CLASS,
        xproto::AtomEnum::STRING,
        b"simple_window\0simple_window\0",
    )?;
    Ok(window)
}

fn gc_font_get<C: Connection>(
    conn: &C,
    screen: &Screen,
    window: Window,
    color: u32,
    font_name: &str,
) -> Result<Gcontext, ReplyOrIdError> {
    let font = conn.generate_id()?;

    conn.open_font(font, font_name.as_bytes())?;

    let gc = conn.generate_id()?;
    let values = CreateGCAux::default()
        .foreground(color)
        .background(screen.black_pixel)
        .font(font);
    conn.create_gc(gc, window, &values)?;

    conn.close_font(font)?;

    Ok(gc)
}

fn text_draw<C: Connection>(
    conn: &C,
    screen: &Screen,
    window: Window,
    x1: i16,
    y1: i16,
    label: &str,
    color: u32,
) -> Result<(), Box<dyn Error>> {
    let gc = gc_font_get(conn, screen, window, color, "7x13")?;

    conn.image_text8(window, gc, x1, y1, label.as_bytes())?;
    conn.free_gc(gc)?;

    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    // The XCB crate requires ownership of the connection, so we need to use it to connect to the
    // X11 server.
    let (conn, screen_num) = xcb::Connection::connect(None)?;
    let screen_num = usize::try_from(screen_num).unwrap();
    // Now get us an x11rb connection using the same underlying libxcb connection
    let conn = {
        let raw_conn = conn.get_raw_conn().cast();
        unsafe { XCBConnection::from_raw_xcb_connection(raw_conn, false) }
    }?;

    conn.prefetch_extension_information(xkb::X11_EXTENSION_NAME)?;
    let atoms = AtomCollection::new(&conn)?;
    let xkb = conn.xkb_use_extension(1, 0)?;
    let atoms = atoms.reply()?;
    let xkb = xkb.reply()?;
    assert!(xkb.supported);

    // TODO: No idea what to pick here. I guess this is asking unnecessarily for too much?
    let events = xkb::EventType::NEW_KEYBOARD_NOTIFY
        | xkb::EventType::MAP_NOTIFY
        | xkb::EventType::STATE_NOTIFY;
    // TODO: No idea what to pick here. I guess this is asking unnecessarily for too much?
    let map_parts = xkb::MapPart::KEY_TYPES
        | xkb::MapPart::KEY_SYMS
        | xkb::MapPart::MODIFIER_MAP
        | xkb::MapPart::EXPLICIT_COMPONENTS
        | xkb::MapPart::KEY_ACTIONS
        | xkb::MapPart::KEY_BEHAVIORS
        | xkb::MapPart::VIRTUAL_MODS
        | xkb::MapPart::VIRTUAL_MOD_MAP;
    conn.xkb_select_events(
        xkb::ID::USE_CORE_KBD.into(),
        0u8.into(),
        events,
        map_parts,
        map_parts,
        &xkb::SelectEventsAux::new(),
    )?;

    let context = xkbc::Context::new(xkbc::CONTEXT_NO_FLAGS);
    let device_id = xkbc::x11::get_core_keyboard_device_id(&conn);
    let keymap = xkbc::x11::keymap_new_from_device(
        &context,
        &conn,
        device_id,
        xkbc::KEYMAP_COMPILE_NO_FLAGS,
    );
    let mut state = xkbc::x11::state_new_from_device(&keymap, &conn, device_id);

    let window = create_window(&conn, screen_num, &atoms)?;
    let screen = &conn.setup().roots[screen_num];
    conn.map_window(window)?;

    let current_focus = conn.get_input_focus()?.reply()?;
    conn.set_input_focus(current_focus.revert_to, window, CURRENT_TIME)?;

    text_draw(&conn, screen, window, 10, 10, "$ ", screen.white_pixel)?;

    conn.flush()?;

    let foreground = conn.generate_id()?;
    let values = CreateGCAux::default()
        .foreground(screen.black_pixel)
        .graphics_exposures(0);
    conn.create_gc(foreground, window, &values)?;

    let mut prompt = "".to_owned();

    loop {
        match conn.wait_for_event()? {
            Event::ClientMessage(event) => {
                let data = event.data.as_data32();
                if event.format == 32 && event.window == window && data[0] == atoms.WM_DELETE_WINDOW
                {
                    println!("Window was asked to close");
                    break;
                }
            }
            Event::XkbStateNotify(event) => {
                if i32::try_from(event.device_id).unwrap() == device_id {
                    // Inform xkbcommon that the keyboard state changed
                    state.update_mask(
                        event.base_mods.into(),
                        event.latched_mods.into(),
                        event.locked_mods.into(),
                        event.base_group.try_into().unwrap(),
                        event.latched_group.try_into().unwrap(),
                        event.locked_group.into(),
                    );
                }
            }
            Event::KeyPress(event) => {
                let (sym, text) = handle_key(event, &state);
                match sym {
                    xkbc::keysyms::KEY_BackSpace => {
                        prompt.pop();
                    }
                    xkbc::keysyms::KEY_Escape => {
                        return Ok(());
                    }
                    xkbc::keysyms::KEY_Return => {
                        _ = add_history_record(&prompt);
                        set_current_dir(home::home_dir().unwrap())?;
                        _ = exec::Command::new("bash").arg("-ci").arg(&prompt).exec();
                        return Ok(());
                    }
                    xkbc::keysyms::KEY_Right => {
                        prompt = get_history_candidate(&prompt);
                    }
                    _ => {
                        prompt += &text;
                    }
                }
            }
            Event::KeyRelease(_) => {}
            event => println!("Ignoring event {event:?}"),
        }
        conn.poly_fill_rectangle(
            window,
            foreground,
            &[Rectangle {
                x: 0,
                y: 0,
                width: 10000,
                height: 10000,
            }],
        )?;
        let gray = conn
            .alloc_color(screen.default_colormap, 32000, 32000, 32000)?
            .reply()?;
        let history_candidate = get_history_candidate(&prompt);
        text_draw(
            &conn,
            screen,
            window,
            10,
            10,
            &format!("$ {history_candidate}"),
            gray.pixel,
        )?;
        text_draw(
            &conn,
            screen,
            window,
            10,
            10,
            &format!("$ {prompt}"),
            screen.white_pixel,
        )?;
        conn.flush()?;
    }

    Ok(())
}
