use image_editor_core::{
    AdjustmentKind, EditorCommand, KeyModifiers, NavigationDirection, RawKeyEvent, RuntimePlatform,
    ShortcutKey, ShortcutResolver,
};
use proptest::prelude::*;

const UNDO: u8 = 0;
const REDO: u8 = 1;
const INCREASE_ADJUSTMENT: u8 = 2;
const DECREASE_ADJUSTMENT: u8 = 3;
const NAVIGATE_LEFT: u8 = 4;
const NAVIGATE_RIGHT: u8 = 5;
const NAVIGATE_HOME: u8 = 6;
const NAVIGATE_END: u8 = 7;
const FLIP_HORIZONTAL: u8 = 8;
const FLIP_VERTICAL: u8 = 9;
const ROTATE_CLOCKWISE: u8 = 10;
const ROTATE_COUNTERCLOCKWISE: u8 = 11;
const ENTER_CROP: u8 = 12;
const FOCUS_BRIGHTNESS: u8 = 13;
const FOCUS_CONTRAST: u8 = 14;
const COMMIT_ADJUSTMENT: u8 = 15;

fn expected_command(intent: u8) -> EditorCommand {
    match intent {
        UNDO => EditorCommand::Undo,
        REDO => EditorCommand::Redo,
        INCREASE_ADJUSTMENT => EditorCommand::IncreaseAdjustment,
        DECREASE_ADJUSTMENT => EditorCommand::DecreaseAdjustment,
        NAVIGATE_LEFT => EditorCommand::Navigate {
            direction: NavigationDirection::Left,
        },
        NAVIGATE_RIGHT => EditorCommand::Navigate {
            direction: NavigationDirection::Right,
        },
        NAVIGATE_HOME => EditorCommand::Navigate {
            direction: NavigationDirection::Home,
        },
        NAVIGATE_END => EditorCommand::Navigate {
            direction: NavigationDirection::End,
        },
        FLIP_HORIZONTAL => EditorCommand::FlipHorizontal,
        FLIP_VERTICAL => EditorCommand::FlipVertical,
        ROTATE_CLOCKWISE => EditorCommand::RotateClockwise90,
        ROTATE_COUNTERCLOCKWISE => EditorCommand::RotateCounterclockwise90,
        ENTER_CROP => EditorCommand::EnterCrop,
        FOCUS_BRIGHTNESS => EditorCommand::FocusAdjustment(AdjustmentKind::Brightness),
        FOCUS_CONTRAST => EditorCommand::FocusAdjustment(AdjustmentKind::Contrast),
        COMMIT_ADJUSTMENT => EditorCommand::CommitAdjustment,
        _ => unreachable!("the generated intent is restricted to defined shortcuts"),
    }
}

fn events_for_intent(intent: u8, uppercase_character: bool) -> (RawKeyEvent, RawKeyEvent) {
    let character = |lowercase: char| {
        ShortcutKey::Character(if uppercase_character {
            lowercase.to_ascii_uppercase()
        } else {
            lowercase
        })
    };
    let plain = KeyModifiers::default();

    match intent {
        UNDO => (
            RawKeyEvent::press(character('z'), KeyModifiers::command()),
            RawKeyEvent::press(character('z'), KeyModifiers::control()),
        ),
        REDO => (
            RawKeyEvent::press(character('z'), KeyModifiers::command().with_shift()),
            RawKeyEvent::press(character('z'), KeyModifiers::control().with_shift()),
        ),
        INCREASE_ADJUSTMENT => (
            RawKeyEvent::press(ShortcutKey::ArrowUp, KeyModifiers::option()),
            RawKeyEvent::press(ShortcutKey::ArrowUp, KeyModifiers::alt()),
        ),
        DECREASE_ADJUSTMENT => (
            RawKeyEvent::press(ShortcutKey::ArrowDown, KeyModifiers::option()),
            RawKeyEvent::press(ShortcutKey::ArrowDown, KeyModifiers::alt()),
        ),
        NAVIGATE_LEFT => (
            RawKeyEvent::press(ShortcutKey::ArrowLeft, plain),
            RawKeyEvent::press(ShortcutKey::ArrowLeft, plain),
        ),
        NAVIGATE_RIGHT => (
            RawKeyEvent::press(ShortcutKey::ArrowRight, plain),
            RawKeyEvent::press(ShortcutKey::ArrowRight, plain),
        ),
        NAVIGATE_HOME => (
            RawKeyEvent::press(ShortcutKey::Home, plain),
            RawKeyEvent::press(ShortcutKey::Home, plain),
        ),
        NAVIGATE_END => (
            RawKeyEvent::press(ShortcutKey::End, plain),
            RawKeyEvent::press(ShortcutKey::End, plain),
        ),
        FLIP_HORIZONTAL => (
            RawKeyEvent::press(character('f'), plain),
            RawKeyEvent::press(character('f'), plain),
        ),
        FLIP_VERTICAL => (
            RawKeyEvent::press(character('f'), plain.with_shift()),
            RawKeyEvent::press(character('f'), plain.with_shift()),
        ),
        ROTATE_CLOCKWISE => (
            RawKeyEvent::press(character('r'), plain),
            RawKeyEvent::press(character('r'), plain),
        ),
        ROTATE_COUNTERCLOCKWISE => (
            RawKeyEvent::press(character('r'), plain.with_shift()),
            RawKeyEvent::press(character('r'), plain.with_shift()),
        ),
        ENTER_CROP => (
            RawKeyEvent::press(character('c'), plain),
            RawKeyEvent::press(character('c'), plain),
        ),
        FOCUS_BRIGHTNESS => (
            RawKeyEvent::press(character('b'), plain),
            RawKeyEvent::press(character('b'), plain),
        ),
        FOCUS_CONTRAST => (
            RawKeyEvent::press(character('d'), plain),
            RawKeyEvent::press(character('d'), plain),
        ),
        COMMIT_ADJUSTMENT => (
            RawKeyEvent::press(ShortcutKey::Enter, plain),
            RawKeyEvent::press(ShortcutKey::Enter, plain),
        ),
        _ => unreachable!("the generated intent is restricted to defined shortcuts"),
    }
}

fn apply_event_variant(mut event: RawKeyEvent, variant: u8) -> RawKeyEvent {
    match variant {
        0 => event,
        1 => {
            event.pressed = false;
            event
        }
        2 => {
            event.repeat = true;
            event
        }
        3 => {
            event.consumed_by_text_control = true;
            event
        }
        _ => unreachable!("the generated event variant is restricted to four states"),
    }
}

// Feature: macos-image-editor, Property 8: Shortcut resolution has platform-invariant semantics
// Validates: Requirements 8.3, 8.5, 8.6, 10.3, 10.4
proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    #[test]
    fn matched_platform_shortcuts_have_equal_semantics_and_ignore_unaccepted_events(
        intent in 0_u8..=COMMIT_ADJUSTMENT,
        variant in 0_u8..4,
        uppercase_character in any::<bool>(),
    ) {
        let expected = (variant == 0).then(|| expected_command(intent));
        let (macos_event, linux_event) = events_for_intent(intent, uppercase_character);
        let macos_result = ShortcutResolver::new(RuntimePlatform::MacOs)
            .resolve(apply_event_variant(macos_event, variant));
        let linux_result = ShortcutResolver::new(RuntimePlatform::Linux)
            .resolve(apply_event_variant(linux_event, variant));

        prop_assert_eq!(macos_result, expected.clone());
        prop_assert_eq!(linux_result, expected);
    }
}
