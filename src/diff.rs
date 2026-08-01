use std::collections::BTreeMap;

use colored::Colorize;
use similar::TextDiff;

fn is_text(data: &[u8]) -> bool {
    let sample = &data[..data.len().min(8192)];
    !sample.contains(&0)
}

pub fn print_maps(
    old_label: &str,
    old: &BTreeMap<String, Vec<u8>>,
    new_label: &str,
    new: &BTreeMap<String, Vec<u8>>,
) {
    let mut paths: Vec<&String> = old.keys().chain(new.keys()).collect();
    paths.sort();
    paths.dedup();

    let mut any = false;
    for path in paths {
        let before = old.get(path);
        let after = new.get(path);
        if before == after {
            continue;
        }
        any = true;
        print_file(old_label, new_label, path, before, after);
    }
    if !any {
        println!("no differences");
    }
}

fn print_file(
    old_label: &str,
    new_label: &str,
    path: &str,
    before: Option<&Vec<u8>>,
    after: Option<&Vec<u8>>,
) {
    match (before, after) {
        (None, Some(_)) => println!("{} {path}", "new file:".green().bold()),
        (Some(_), None) => println!("{} {path}", "deleted:".red().bold()),
        _ => println!("{} {path}", "changed:".yellow().bold()),
    }

    let empty = Vec::new();
    let a = before.unwrap_or(&empty);
    let b = after.unwrap_or(&empty);
    if !is_text(a) || !is_text(b) {
        println!("  {}", "binary files differ".dimmed());
        return;
    }
    let (Ok(a_text), Ok(b_text)) = (std::str::from_utf8(a), std::str::from_utf8(b)) else {
        println!("  {}", "binary files differ".dimmed());
        return;
    };
    let diff = TextDiff::from_lines(a_text, b_text);
    let rendered = diff
        .unified_diff()
        .header(
            &format!("{old_label}/{path}"),
            &format!("{new_label}/{path}"),
        )
        .to_string();
    for (i, line) in rendered.lines().enumerate() {
        let styled = if i < 2 {
            line.bold()
        } else if line.starts_with('+') {
            line.green()
        } else if line.starts_with('-') {
            line.red()
        } else {
            line.dimmed()
        };
        println!("{styled}");
    }
    println!();
}
