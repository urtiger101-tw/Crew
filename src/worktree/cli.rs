//! Resolve relative paths in the caller, not in the detached server.
use super::{ApiRequest, HELP, Request};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub fn parse(sub: &str, args: &[String]) -> Result<ApiRequest, String> {
    let allowed: &[&str] = match sub {
        "list" => &["repo", "cwd"],
        "create" => &[
            "repo", "cwd", "path", "branch", "base", "name", "label", "focus", "no-focus",
            "no-open",
        ],
        "open" => &[
            "repo", "cwd", "path", "branch", "name", "label", "focus", "no-focus",
        ],
        "remove" => &["repo", "cwd", "path"],
        _ => return Err(HELP.into()),
    };
    let opts = options(args, allowed)?;
    let repo = absolute(one_of(&opts, "repo", "cwd")?.unwrap_or("."))?;
    let name = one_of(&opts, "name", "label")?.map(str::to_string);
    super::validate_name(name.as_deref()).map_err(|e| e.message)?;
    if opts.contains_key("focus") && opts.contains_key("no-focus") {
        return Err("use only one of --focus and --no-focus".into());
    }
    let focus = opts.contains_key("focus");
    let req = match sub {
        "list" => Request::List { repo },
        "create" => Request::Create {
            repo,
            path: absolute(required(&opts, "path")?)?,
            branch: required(&opts, "branch")?.to_string(),
            base: opts.get("base").cloned(),
            name,
            focus,
            open: !opts.contains_key("no-open"),
        },
        "open" => {
            let (path, branch) = (opts.get("path"), opts.get("branch").cloned());
            if path.is_some() == branch.is_some() {
                return Err("worktree open requires exactly one of --path and --branch".into());
            }
            Request::Open {
                repo,
                path: path.map(|p| absolute(p)).transpose()?,
                branch,
                name,
                focus,
            }
        }
        "remove" => Request::Remove {
            repo,
            path: absolute(required(&opts, "path")?)?,
        },
        _ => unreachable!(),
    };
    Ok(ApiRequest::Worktree(req))
}
fn required<'a>(opts: &'a BTreeMap<String, String>, key: &str) -> Result<&'a str, String> {
    opts.get(key)
        .map(String::as_str)
        .ok_or_else(|| format!("missing --{key}"))
}
fn one_of<'a>(
    opts: &'a BTreeMap<String, String>,
    a: &str,
    b: &str,
) -> Result<Option<&'a str>, String> {
    if opts.contains_key(a) && opts.contains_key(b) {
        return Err(format!("use only one of --{a} and --{b}"));
    }
    Ok(opts.get(a).or_else(|| opts.get(b)).map(String::as_str))
}
fn absolute(value: &str) -> Result<String, String> {
    if value.is_empty() || value.contains('\0') {
        return Err("path must not be empty or contain NUL".into());
    }
    let path = Path::new(value);
    let path: PathBuf = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| format!("cannot resolve current directory: {e}"))?
            .join(path)
    };
    path.into_os_string()
        .into_string()
        .map_err(|_| "path must be valid UTF-8".into())
}
fn options(args: &[String], allowed: &[&str]) -> Result<BTreeMap<String, String>, String> {
    let mut opts = BTreeMap::new();
    let mut i = 0;
    while let Some(arg) = args.get(i) {
        let body = arg
            .strip_prefix("--")
            .ok_or_else(|| format!("unexpected argument: {arg}"))?;
        let (key, inline) = body
            .split_once('=')
            .map_or((body, None), |(k, v)| (k, Some(v)));
        if !allowed.contains(&key) {
            return Err(format!("unknown option: --{key}"));
        }
        if opts.contains_key(key) {
            return Err(format!("duplicate option: --{key}"));
        }
        i += 1;
        let flag = matches!(key, "focus" | "no-focus" | "no-open");
        let value = if flag {
            if inline.is_some() {
                return Err(format!("--{key} takes no value"));
            }
            String::new()
        } else if let Some(value) = inline {
            value.to_string()
        } else {
            let value = args
                .get(i)
                .filter(|s| !s.starts_with("--"))
                .ok_or_else(|| format!("missing value for --{key}"))?;
            i += 1;
            value.clone()
        };
        if !flag && (value.is_empty() || value.contains('\0')) {
            return Err(format!("--{key} must not be empty or contain NUL"));
        }
        opts.insert(key.to_string(), value);
    }
    Ok(opts)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|s| s.to_string()).collect()
    }
    #[test]
    fn paths_are_absolutized_and_names_preserved() {
        let r = parse(
            "create",
            &args(&[
                "--path",
                "work tree 中文",
                "--branch",
                "feat/test",
                "--name",
                "my worker",
                "--no-open",
            ]),
        )
        .unwrap();
        let ApiRequest::Worktree(Request::Create {
            path, name, open, ..
        }) = r
        else {
            panic!()
        };
        assert!(Path::new(&path).is_absolute());
        assert!(path.ends_with("work tree 中文"));
        assert_eq!(name.as_deref(), Some("my worker"));
        assert!(!open);
    }
    #[test]
    fn rejects_ambiguous_options_and_unsafe_shape() {
        for words in [
            vec!["--path", "a", "--path", "b"],
            vec!["--path", "a", "--branch", "b"],
            vec!["--path", "a", "--repo", "a", "--cwd", "b"],
            vec!["--path", "a", "--focus", "--no-focus"],
            vec!["--path", "a", "--focus=true"],
            vec!["--path", "a", "--name", "bad\nname"],
        ] {
            assert!(parse("open", &args(&words)).is_err(), "{words:?}");
        }
        assert!(parse("remove", &args(&["--path", "a", "--force"])).is_err());
    }
}
