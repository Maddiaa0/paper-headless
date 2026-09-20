#[path = "../tests/support/mod.rs"]
mod support;
pub(crate) use support::*;

pub(crate) fn settings(temp: &Temp) -> crate::settings::Settings {
    crate::settings::Settings::resolve(
        Some(temp.path.clone()),
        ":99".into(),
        "1600x1000x24".into(),
        9222,
        29979,
        Some(temp.path.join("paper")),
    )
    .unwrap()
}
