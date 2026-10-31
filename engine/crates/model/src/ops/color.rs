//! The operations on colour management (see [`crate::color_management`]): the settings of the
//! project, and the colour of media set by hand.

use super::{missing, swap, Op, OpError};
use crate::color_management::{ColorSettings, Levels};
use crate::grade::{changed, merge};
use crate::project::{Id, Project};

/// The colour settings as they are written.
fn written(settings: &ColorSettings) -> serde_json::Value {
    serde_json::to_value(settings).expect("colour settings are written as JSON")
}

/// Set the colour management of the project by `patch`, and give back the patch that sets it
/// back.
///
/// What the patch makes must be colour settings (`color_settings`, with serde's message), each
/// of them one that it may be ([`ColorSettings::check`]), whether it applies or not; and each
/// setting in the patch must apply to the settings before it or after it (`inapplicable`). The
/// rule takes both, so that the inverse of a change, which sets back what the change set, is
/// never refused.
pub(super) fn set_color_settings(
    project: &mut Project,
    patch: &serde_json::Value,
) -> Result<Op, OpError> {
    let Some(fields) = patch.as_object() else {
        return Err(OpError::ColorSettings {
            why: "the settings are not an object".into(),
        });
    };
    let before = written(&project.color);
    let mut value = before.clone();
    merge(&mut value, patch);
    let settings: ColorSettings =
        serde_json::from_value(value).map_err(|error| OpError::ColorSettings {
            why: error.to_string(),
        })?;
    settings.check()?;
    if let Some(name) = fields
        .keys()
        .find(|name| !(project.color.applies(name) || settings.applies(name)))
    {
        return Err(OpError::Inapplicable {
            setting: name.clone(),
        });
    }
    project.color = settings;
    Ok(Op::SetColorSettings {
        settings: changed(&before, patch),
    })
}

/// [`Op::SetColorSettings`] as it was done: the patch with the values that the project has now
/// at its paths, as the engine writes them (`500` as `500.0`).
pub(super) fn done(project: &Project, patch: &serde_json::Value) -> Op {
    Op::SetColorSettings {
        settings: changed(&written(&project.color), patch),
    }
}

/// Set the input colour space and the data levels of media that are given.
#[allow(clippy::option_option, reason = "left out and null are told apart")]
pub(super) fn set_media_color(
    project: &mut Project,
    id: &Id,
    input: Option<Option<String>>,
    levels: Option<Levels>,
) -> Result<Op, OpError> {
    let at = project
        .media
        .iter()
        .position(|m| m.id == *id)
        .ok_or_else(|| missing("media", id))?;
    let old = &project.media[at];
    let mut new = old.clone();
    swap(&mut new.color_override.input, input.clone());
    swap(&mut new.color_override.levels, levels);
    crate::color_management::check_override(&new)?;
    let inverse = Op::SetMediaColor {
        id: id.clone(),
        input: input.map(|_| old.color_override.input.clone()),
        levels: levels.map(|_| old.color_override.levels),
    };
    project.media[at] = new;
    Ok(inverse)
}
