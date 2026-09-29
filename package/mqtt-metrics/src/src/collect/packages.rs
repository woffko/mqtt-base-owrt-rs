// SPDX-License-Identifier: GPL-2.0-or-later
//! Number of upgradable packages from the locally cached package index.
//! The collector never refreshes the index (`apk update`), so it causes no
//! network traffic; the value is as fresh as the last manual update.

use super::{Context, GroupResult, Reading, Value};
use crate::catalog::*;
use crate::source::SourceError;

pub fn collect(ctx: &Context, out: &mut GroupResult) {
    let listing = match ctx.source.command("apk", &["list", "--upgradable"]) {
        Ok(text) => Ok(text),
        Err(SourceError::Missing(_)) => ctx.source.command("opkg", &["list-upgradable"]),
        Err(e) => Err(e),
    };
    match listing {
        Ok(text) => {
            let count = text.lines().filter(|l| !l.trim().is_empty()).count();
            out.push(
                &PACKAGES_UPGRADABLE,
                Instance::none(),
                Reading::Value(Value::Int(i64::try_from(count).unwrap_or(i64::MAX))),
            );
        }
        Err(SourceError::Missing(_)) => out.miss(&PACKAGES_UPGRADABLE, "no package manager found"),
        Err(e) => out.push(
            &PACKAGES_UPGRADABLE,
            Instance::none(),
            Reading::from_error(&e),
        ),
    }
}
