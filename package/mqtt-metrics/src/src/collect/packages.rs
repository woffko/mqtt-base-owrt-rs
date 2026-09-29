// SPDX-License-Identifier: GPL-2.0-or-later
//! Number of upgradable packages from the locally cached package index.
//! The collector never refreshes the index (`apk update`), so it causes no
//! network traffic; the value is as fresh as the last manual update. The cache
//! lives under `/var`, which is empty after every boot until someone runs
//! `apk update`; `apk list --upgradable` then exits 0 with no output, and 0
//! would be a wrong answer.

use super::{Context, GroupResult, Reading, Value};
use crate::catalog::*;
use crate::source::SourceError;

/// Where `apk` keeps the package indexes it downloaded.
const APK_CACHE: &str = "/var/cache/apk";

pub fn collect(ctx: &Context, out: &mut GroupResult) {
    let listing = match ctx.source.command("apk", &["list", "--upgradable"]) {
        Ok(text) => {
            let cached = ctx
                .source
                .list_dir(APK_CACHE)
                .is_ok_and(|names| names.iter().any(|n| n.starts_with("APKINDEX.")));
            if cached {
                Ok(text)
            } else {
                out.push(
                    &PACKAGES_UPGRADABLE,
                    Instance::none(),
                    Reading::missing("no package index is cached; run apk update"),
                );
                return;
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::tests::{fixture, run, value_of};
    use crate::collect::CollectorState;
    use crate::config::Config;
    use crate::source::{Source, SourceResult};
    use serde_json::Value as Json;

    /// A router with `apk`, and a cache directory that has these entries.
    struct Apk {
        cache: Vec<String>,
    }

    impl Source for Apk {
        fn ubus(&self, object: &str, _: &str, _: Option<&str>) -> SourceResult<Json> {
            Err(SourceError::Missing(format!("ubus object {object}")))
        }
        fn read(&self, path: &str) -> SourceResult<String> {
            Err(SourceError::Missing(path.to_string()))
        }
        fn list_dir(&self, path: &str) -> SourceResult<Vec<String>> {
            if path == APK_CACHE {
                Ok(self.cache.clone())
            } else {
                Err(SourceError::Missing(path.to_string()))
            }
        }
        fn command(&self, program: &str, _: &[&str]) -> SourceResult<String> {
            if program == "apk" {
                // What apk prints on stdout when nothing is cached: nothing.
                Ok(if self.cache.is_empty() {
                    String::new()
                } else {
                    "curl-8.0 x86_64 {curl} [upgradable from: curl-7.9]\nnano-7 x86_64 {nano}\n"
                        .to_string()
                })
            } else {
                Err(SourceError::Missing(program.to_string()))
            }
        }
        fn now(&self) -> u64 {
            0
        }
    }

    fn count(src: &dyn Source) -> Option<crate::collect::Reading> {
        let r = run(
            Group::Packages,
            src,
            &Config::default(),
            &mut CollectorState::default(),
            0.0,
        );
        value_of(&r, "packages.upgradable", "", "")
    }

    #[test]
    fn no_cached_index_is_unavailable_not_zero() {
        let none = Apk { cache: Vec::new() };
        assert!(matches!(
            count(&none),
            Some(Reading::Unavailable {
                permanent: false,
                failed: false,
                ..
            })
        ));
        let cached = Apk {
            cache: vec!["APKINDEX.302fd236.tar.gz".to_string()],
        };
        assert_eq!(count(&cached), Some(Reading::Value(Value::Int(2))));
    }

    #[test]
    fn a_router_without_a_package_manager_has_no_such_metric() {
        let src = fixture("primary");
        assert_eq!(count(&src), None);
    }
}
