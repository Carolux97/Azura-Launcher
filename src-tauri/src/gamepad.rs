//! Controller input for navigating the launcher.
//!
//! The interface never reads controllers itself (the webview's Gamepad API depends on how each
//! platform's WebKit was built): the app reads them and sends one `gamepad-input` event per action
//! pressed or released (`{"action": "up", "pressed": true, "family": "xbox"}`). Each platform has
//! the reader that sees every controller there:
//! - macOS: Apple's GameController framework (`apple`). It is the one that receives a DualSense's
//!   input over Bluetooth; reading the HID device directly gets only its connection.
//! - Linux: gilrs (`gilrs_reader`), on evdev.
//!
//! Buttons are named by position: South (A, Cross) confirms and East (B, Circle) goes back on every
//! controller, West and North serve the on-screen keyboard; `family` only picks the glyphs the
//! interface draws for them. Presses are sent only while the launcher window has the focus, so a
//! controller playing the game (in its own window) does not also move the launcher; releases are
//! always sent, so a direction held while the focus leaves never keeps repeating.

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

/// Event carrying one action pressed or released.
pub const INPUT_EVENT: &str = "gamepad-input";

/// How far the left stick must go to press a direction, and how far back to release it. The gap
/// keeps a stick resting near the threshold from pressing again and again.
const STICK_PRESS: f32 = 0.6;
const STICK_RELEASE: f32 = 0.35;

/// The button layout a controller's glyphs follow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Family {
    /// Xbox controllers, and the Steam Deck, whose face buttons are labelled the same way.
    Xbox,
    PlayStation,
    Nintendo,
    Generic,
}

/// A controller button the launcher uses, by position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Pad {
    Up,
    Down,
    Left,
    Right,
    South,
    East,
    West,
    North,
    LeftShoulder,
    RightShoulder,
    Start,
}

/// A stick axis: horizontal or vertical, up positive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StickAxis {
    X,
    Y,
}

/// One action pressed or released, sent to the interface as JSON.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Input {
    pub action: &'static str,
    pub pressed: bool,
    pub family: Family,
}

/// The family of a controller from its USB vendor ID, or else its name (or, on macOS, its
/// product category: "DualSense", "Xbox One", "Switch Pro Controller", ...).
pub fn family(vendor_id: Option<u16>, name: &str) -> Family {
    match vendor_id {
        Some(0x045e) | Some(0x28de) => return Family::Xbox,
        Some(0x054c) => return Family::PlayStation,
        Some(0x057e) => return Family::Nintendo,
        _ => {}
    }
    let name = name.to_ascii_lowercase();
    if name.contains("xbox") || name.contains("steam deck") {
        Family::Xbox
    } else if ["dualsense", "dualshock", "playstation", "ps4", "ps5"].iter().any(|n| name.contains(n)) {
        Family::PlayStation
    } else if ["pro controller", "joy-con", "nintendo", "switch"].iter().any(|n| name.contains(n)) {
        Family::Nintendo
    } else {
        Family::Generic
    }
}

/// The launcher action of a button.
pub fn action(button: Pad) -> &'static str {
    match button {
        Pad::Up => "up",
        Pad::Down => "down",
        Pad::Left => "left",
        Pad::Right => "right",
        Pad::South => "confirm",
        Pad::East => "back",
        Pad::West => "west",
        Pad::North => "north",
        Pad::LeftShoulder => "previous",
        Pad::RightShoulder => "next",
        Pad::Start => "start",
    }
}

/// Turns the left stick's position into direction presses and releases, one direction per axis.
#[derive(Debug, Default)]
pub struct Stick {
    /// The direction each axis holds pressed: (horizontal, vertical).
    held: (Option<&'static str>, Option<&'static str>),
}

impl Stick {
    /// The presses and releases a new value of `axis` causes.
    pub fn update(&mut self, axis: StickAxis, value: f32) -> Vec<(&'static str, bool)> {
        let (slot, negative, positive) = match axis {
            StickAxis::X => (&mut self.held.0, "left", "right"),
            StickAxis::Y => (&mut self.held.1, "down", "up"),
        };
        let wanted = if value >= STICK_PRESS {
            Some(positive)
        } else if value <= -STICK_PRESS {
            Some(negative)
        } else if value.abs() < STICK_RELEASE {
            None
        } else {
            // Between the two thresholds the stick keeps what it holds.
            return Vec::new();
        };
        if wanted == *slot {
            return Vec::new();
        }
        let mut changes = Vec::new();
        if let Some(released) = slot.take() {
            changes.push((released, false));
        }
        if let Some(pressed) = wanted {
            changes.push((pressed, true));
            *slot = Some(pressed);
        }
        changes
    }
}

/// Sends the interface the actions in `changes`: presses only while the launcher window has the
/// focus, releases always.
fn send(app: &AppHandle, family: Family, changes: impl IntoIterator<Item = (&'static str, bool)>) {
    // While the game runs the controllers are the game's: the launcher's interface is not woken
    // for them at all (the interface ignores them then anyway).
    if app.try_state::<crate::runtime::GameProcess>().is_some_and(|process| process.is_running()) {
        return;
    }
    let focused = || app.get_webview_window("main").and_then(|window| window.is_focused().ok()).unwrap_or(false);
    for (action, pressed) in changes {
        if pressed && !focused() {
            continue;
        }
        let _ = app.emit(INPUT_EVENT, Input { action, pressed, family });
    }
}

/// Starts reading the controllers. On macOS it must run on the main thread (the app's setup), where
/// the GameController framework delivers its notifications and input.
pub fn spawn(app: AppHandle) {
    #[cfg(target_os = "macos")]
    apple::start(app);
    #[cfg(not(target_os = "macos"))]
    gilrs_reader::start(app);
}

/// The GameController framework: every controller Apple supports (Xbox, DualSense, DualShock,
/// Switch Pro, MFi), each with its product category. It delivers input only while the app is in
/// the foreground, on the main queue.
#[cfg(target_os = "macos")]
mod apple {
    use std::collections::HashMap;
    use std::ptr::NonNull;
    use std::sync::{Arc, Mutex};

    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2_foundation::{NSNotification, NSNotificationCenter};
    use objc2_game_controller::{
        GCController, GCControllerDidConnectNotification, GCControllerDidDisconnectNotification, GCControllerElement, GCDevice,
        GCExtendedGamepad,
    };
    use tauri::AppHandle;

    use super::{action, family, send, Family, Pad, Stick, StickAxis};

    /// What a controller held at its last report, to send only what changed.
    #[derive(Default)]
    struct Held {
        buttons: Vec<Pad>,
        stick: Stick,
    }

    /// The buttons `pad` holds down now.
    fn pressed(pad: &GCExtendedGamepad) -> Vec<Pad> {
        // SAFETY: plain property reads of a live gamepad profile, on the main queue the framework
        // calls back on.
        unsafe {
            let dpad = pad.dpad();
            [
                (Pad::Up, dpad.up().isPressed()),
                (Pad::Down, dpad.down().isPressed()),
                (Pad::Left, dpad.left().isPressed()),
                (Pad::Right, dpad.right().isPressed()),
                (Pad::South, pad.buttonA().isPressed()),
                (Pad::East, pad.buttonB().isPressed()),
                (Pad::West, pad.buttonX().isPressed()),
                (Pad::North, pad.buttonY().isPressed()),
                (Pad::LeftShoulder, pad.leftShoulder().isPressed()),
                (Pad::RightShoulder, pad.rightShoulder().isPressed()),
                (Pad::Start, pad.buttonMenu().isPressed()),
            ]
            .into_iter()
            .filter_map(|(button, down)| down.then_some(button))
            .collect()
        }
    }

    /// The family of a controller from its product category, or else its vendor name.
    fn family_of(controller: &GCController) -> Family {
        // SAFETY: plain property reads of a connected controller.
        let (category, vendor) = unsafe {
            (controller.productCategory().to_string(), controller.vendorName().map(|name| name.to_string()))
        };
        match family(None, &category) {
            Family::Generic => family(None, vendor.as_deref().unwrap_or_default()),
            known => known,
        }
    }

    /// The key a controller's state is kept under while it is connected.
    fn key(controller: &GCController) -> usize {
        controller as *const GCController as usize
    }

    /// Follows one controller's input: on every change of its gamepad profile, sends the buttons
    /// newly pressed and released, and what the left stick did.
    fn watch(app: &AppHandle, controller: &GCController, held: &Arc<Mutex<HashMap<usize, Held>>>) {
        // SAFETY: reading the profile of a connected controller.
        let Some(pad) = (unsafe { controller.extendedGamepad() }) else { return };
        let family = family_of(controller);
        let key = key(controller);
        held.lock().unwrap().insert(key, Held::default());
        let (app, held) = (app.clone(), held.clone());
        let handler = RcBlock::new(move |pad: NonNull<GCExtendedGamepad>, _element: NonNull<GCControllerElement>| {
            // SAFETY: the framework passes the live profile that changed.
            let pad = unsafe { pad.as_ref() };
            let now = pressed(pad);
            // SAFETY: plain property reads, as in `pressed`.
            let (x, y) = unsafe {
                let stick = pad.leftThumbstick();
                (stick.xAxis().value(), stick.yAxis().value())
            };
            let mut changes = Vec::new();
            {
                let mut all = held.lock().unwrap();
                let state = all.entry(key).or_default();
                changes.extend(state.buttons.iter().filter(|b| !now.contains(b)).map(|b| (action(*b), false)));
                changes.extend(now.iter().filter(|b| !state.buttons.contains(b)).map(|b| (action(*b), true)));
                changes.extend(state.stick.update(StickAxis::X, x));
                changes.extend(state.stick.update(StickAxis::Y, y));
                state.buttons = now;
            }
            send(&app, family, changes);
        });
        // SAFETY: the profile copies the block and calls it on the main queue.
        unsafe { pad.setValueChangedHandler(RcBlock::as_ptr(&handler)) };
    }

    /// The controller a connection notification is about.
    fn controller_of(note: NonNull<NSNotification>) -> Option<Retained<GCController>> {
        // SAFETY: the framework posts these notifications with the GCController as their object.
        unsafe {
            let object: Retained<AnyObject> = note.as_ref().object()?;
            Some(Retained::cast_unchecked(object))
        }
    }

    pub fn start(app: AppHandle) {
        let held: Arc<Mutex<HashMap<usize, Held>>> = Arc::default();
        // SAFETY: the framework's class methods and notification names, used on the main thread.
        unsafe {
            for controller in GCController::controllers() {
                watch(&app, &controller, &held);
            }
            let center = NSNotificationCenter::defaultCenter();
            let (connect_app, connect_held) = (app.clone(), held.clone());
            let connected = RcBlock::new(move |note: NonNull<NSNotification>| {
                if let Some(controller) = controller_of(note) {
                    watch(&connect_app, &controller, &connect_held);
                }
            });
            let disconnected = RcBlock::new(move |note: NonNull<NSNotification>| {
                if let Some(controller) = controller_of(note) {
                    // What it held is released, so nothing keeps repeating.
                    if let Some(state) = held.lock().unwrap().remove(&key(&controller)) {
                        send(&app, family_of(&controller), state.buttons.into_iter().map(|b| (action(b), false)));
                    }
                }
            });
            // The observers live as long as the app.
            std::mem::forget(center.addObserverForName_object_queue_usingBlock(
                Some(GCControllerDidConnectNotification),
                None,
                None,
                &connected,
            ));
            std::mem::forget(center.addObserverForName_object_queue_usingBlock(
                Some(GCControllerDidDisconnectNotification),
                None,
                None,
                &disconnected,
            ));
        }
    }
}

/// gilrs on evdev: every controller the kernel exposes, each with its USB vendor ID.
#[cfg(not(target_os = "macos"))]
mod gilrs_reader {
    use std::thread;
    use std::time::Duration;

    use gilrs::{Axis, Button, EventType, Gilrs};
    use tauri::AppHandle;

    use super::{action, family, send, Pad, Stick, StickAxis};

    /// The launcher button of a gilrs button. gilrs names the shoulder buttons LeftTrigger and
    /// RightTrigger (LB and RB); the analog triggers are LeftTrigger2 and RightTrigger2.
    pub fn pad(button: Button) -> Option<Pad> {
        Some(match button {
            Button::DPadUp => Pad::Up,
            Button::DPadDown => Pad::Down,
            Button::DPadLeft => Pad::Left,
            Button::DPadRight => Pad::Right,
            Button::South => Pad::South,
            Button::East => Pad::East,
            Button::West => Pad::West,
            Button::North => Pad::North,
            Button::LeftTrigger => Pad::LeftShoulder,
            Button::RightTrigger => Pad::RightShoulder,
            Button::Start => Pad::Start,
            _ => return None,
        })
    }

    pub fn start(app: AppHandle) {
        thread::spawn(move || {
            let mut gilrs = match Gilrs::new() {
                Ok(gilrs) => gilrs,
                Err(error) => {
                    eprintln!("Controllers are not available: {error}");
                    return;
                }
            };
            let mut stick = Stick::default();
            loop {
                let Some(event) = gilrs.next_event_blocking(Some(Duration::from_millis(250))) else {
                    continue;
                };
                let gamepad = gilrs.gamepad(event.id);
                let family = family(gamepad.vendor_id(), gamepad.name());
                let changes = match event.event {
                    EventType::ButtonPressed(button, _) => pad(button).map(|b| vec![(action(b), true)]).unwrap_or_default(),
                    EventType::ButtonReleased(button, _) => pad(button).map(|b| vec![(action(b), false)]).unwrap_or_default(),
                    EventType::AxisChanged(Axis::LeftStickX, value, _) => stick.update(StickAxis::X, value),
                    EventType::AxisChanged(Axis::LeftStickY, value, _) => stick.update(StickAxis::Y, value),
                    _ => Vec::new(),
                };
                send(&app, family, changes);
            }
        });
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn maps_gilrs_buttons_by_position() {
            assert_eq!(pad(Button::South), Some(Pad::South));
            assert_eq!(pad(Button::LeftTrigger), Some(Pad::LeftShoulder));
            assert_eq!(pad(Button::LeftTrigger2), None);
        }

        /// Prints every event the controllers send for `AZURA_PAD_SECONDS` seconds (default 30),
        /// with the controller's name and how gilrs maps it: `cargo test prints_controller_events
        /// -- --ignored --nocapture`, to see what a controller's buttons arrive as.
        #[test]
        #[ignore]
        fn prints_controller_events() {
            let seconds = std::env::var("AZURA_PAD_SECONDS").ok().and_then(|s| s.parse().ok()).unwrap_or(30);
            let mut gilrs = Gilrs::new().expect("controller support");
            let started = std::time::Instant::now();
            while started.elapsed() < Duration::from_secs(seconds) {
                while let Some(event) = gilrs.next_event() {
                    let gamepad = gilrs.gamepad(event.id);
                    println!("{} [{:?}]: {:?}", gamepad.name(), gamepad.mapping_source(), event.event);
                }
                thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tells_the_controller_families_apart() {
        assert_eq!(family(Some(0x045e), "Xbox Wireless Controller"), Family::Xbox);
        assert_eq!(family(Some(0x28de), "Steam Deck"), Family::Xbox);
        assert_eq!(family(Some(0x054c), "DualSense Wireless Controller"), Family::PlayStation);
        assert_eq!(family(Some(0x057e), "Pro Controller"), Family::Nintendo);
        // Without a vendor ID (macOS, some Bluetooth connections), the name or category decides.
        assert_eq!(family(None, "DualSense"), Family::PlayStation);
        assert_eq!(family(None, "Xbox One"), Family::Xbox);
        assert_eq!(family(None, "Switch Pro Controller"), Family::Nintendo);
        assert_eq!(family(None, "8BitDo SN30 Pro"), Family::Generic);
    }

    #[test]
    fn maps_buttons_by_position() {
        assert_eq!(action(Pad::South), "confirm");
        assert_eq!(action(Pad::East), "back");
        assert_eq!(action(Pad::West), "west");
        assert_eq!(action(Pad::North), "north");
        assert_eq!(action(Pad::LeftShoulder), "previous");
        assert_eq!(action(Pad::RightShoulder), "next");
        assert_eq!(action(Pad::Up), "up");
    }

    #[test]
    fn the_stick_presses_and_releases_directions_with_a_gap() {
        let mut stick = Stick::default();
        assert_eq!(stick.update(StickAxis::Y, 0.4), vec![]);
        assert_eq!(stick.update(StickAxis::Y, 0.7), vec![("up", true)]);
        // Wobbling between the thresholds keeps it pressed.
        assert_eq!(stick.update(StickAxis::Y, 0.5), vec![]);
        assert_eq!(stick.update(StickAxis::Y, 0.1), vec![("up", false)]);
        // Straight across to the other side releases one and presses the other.
        assert_eq!(stick.update(StickAxis::X, -0.9), vec![("left", true)]);
        assert_eq!(stick.update(StickAxis::X, 0.9), vec![("left", false), ("right", true)]);
    }
}
