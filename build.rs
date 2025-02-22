//! Build program to generate a program which runs all the testsuites.
//!
//! By generating a separate `#[test]` test for each file, we allow cargo test
//! to automatically run the files in parallel.
//!
//! Idea adapted from: https://github.com/CraneStation/wasmtime/blob/master/build.rs
//! Thanks @sunfishcode

use std::env;
use std::fs::{read_dir, DirEntry, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

fn main() {
    let out_dir =
        PathBuf::from(env::var("OUT_DIR").expect("The OUT_DIR environment variable must be set"));
    let mut out = File::create(out_dir.join("misc_testsuite_tests.rs"))
        .expect("error generating test source file");

    test_directory(&mut out, "misc_testsuite").expect("generating tests");
}

fn test_directory(out: &mut File, testsuite: &str) -> io::Result<()> {
    let mut dir_entries: Vec<_> = read_dir(testsuite)
        .expect("reading testsuite directory")
        .map(|r| r.expect("reading testsuite directory entry"))
        .filter(|dir_entry| {
            let p = dir_entry.path();
            if let Some(ext) = p.extension() {
                // Only look at wast files.
                if ext == "wasm" {
                    // Ignore files starting with `.`, which could be editor temporary files
                    if let Some(stem) = p.file_stem() {
                        if let Some(stemstr) = stem.to_str() {
                            if !stemstr.starts_with('.') {
                                return true;
                            }
                        }
                    }
                }
            }
            false
        })
        .collect();

    dir_entries.sort_by_key(|dir| dir.path());

    writeln!(
        out,
        "mod {} {{",
        Path::new(testsuite)
            .file_stem()
            .expect("testsuite filename should have a stem")
            .to_str()
            .expect("testsuite filename should be representable as a string")
            .replace("-", "_")
    )?;
    writeln!(out, "    use super::{{runtime, utils, setup_log}};")?;
    for dir_entry in dir_entries {
        write_testsuite_tests(out, dir_entry, testsuite)?;
    }
    writeln!(out, "}}")?;
    Ok(())
}

fn write_testsuite_tests(out: &mut File, dir_entry: DirEntry, testsuite: &str) -> io::Result<()> {
    let path = dir_entry.path();
    let stemstr = path
        .file_stem()
        .expect("file_stem")
        .to_str()
        .expect("to_str");

    writeln!(out, "    #[test]")?;
    if ignore(testsuite, stemstr) {
        writeln!(out, "    #[ignore]")?;
    }
    writeln!(
        out,
        "    fn {}() -> Result<(), String> {{",
        avoid_keywords(&stemstr.replace("-", "_"))
    )?;
    write!(out, "        setup_log();")?;
    write!(out, "        let path = std::path::Path::new(\"")?;
    // Write out the string with escape_debug to prevent special characters such
    // as backslash from being reinterpreted.
    for c in path.display().to_string().chars() {
        write!(out, "{}", c.escape_debug())?;
    }
    writeln!(out, "\");")?;
    writeln!(out, "        let data = utils::read_wasm(path)?;")?;
    writeln!(
        out,
        "        let bin_name = utils::extract_exec_name_from_path(path)?;"
    )?;
    let workspace = if no_preopens(testsuite, stemstr) {
        "None"
    } else {
        "Some(&utils::prepare_workspace(&bin_name)?)"
    };
    writeln!(
        out,
        "        runtime::instantiate(&data, &bin_name, {})",
        workspace
    )?;
    writeln!(out, "    }}")?;
    writeln!(out)?;
    Ok(())
}

/// Rename tests which have the same name as Rust keywords.
fn avoid_keywords(name: &str) -> &str {
    match name {
        "if" => "if_",
        "loop" => "loop_",
        "type" => "type_",
        "const" => "const_",
        "return" => "return_",
        other => other,
    }
}

cfg_if::cfg_if! {
    if #[cfg(not(windows))] {
        /// Ignore tests that aren't supported yet.
        fn ignore(_testsuite: &str, _name: &str) -> bool {
            false
        }
    } else {
        /// Ignore tests that aren't supported yet.
        fn ignore(testsuite: &str, name: &str) -> bool {
            if testsuite == "misc_testsuite" {
                match name {
                    "big_random_buf" => false,
                    "sched_yield" => false,
                    "file_pread_pwrite" => false,
                    _ => true,
                }
            } else {
                unreachable!()
            }
        }
    }
}

/// Mark tests which do not require preopens
fn no_preopens(testsuite: &str, name: &str) -> bool {
    if testsuite == "misc_testsuite" {
        match name {
            "big_random_buf" => true,
            "clock_time_get" => true,
            "sched_yield" => true,
            _ => false,
        }
    } else {
        unreachable!()
    }
}
