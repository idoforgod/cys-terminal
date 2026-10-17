// Source pin for the currently private sampler in this single handlers module.
// This lexer is intentionally not a Rust parser or macro-expansion/whole-program proof.
// It ignores comments/literals, checks the cfg(test) tests module boundary (only further
// cfg(test) modules may follow it), and
// fails closed if production submodules/include! would move callers outside its scan.
fn c8_sampler_tokens(source: &str) -> Result<Vec<&str>, String> {
    let b = source.as_bytes(); let mut i = 0; let mut tokens = Vec::new();
    while i < b.len() {
        if b[i].is_ascii_whitespace() { i += 1; continue; }
        if b[i..].starts_with(b"//") {
            i += 2; while i < b.len() && b[i] != b'\n' { i += 1; } continue;
        }
        if b[i..].starts_with(b"/*") {
            let mut depth = 1usize; i += 2;
            while i < b.len() && depth > 0 {
                if b[i..].starts_with(b"/*") { depth += 1; i += 2; }
                else if b[i..].starts_with(b"*/") { depth -= 1; i += 2; }
                else { i += 1; }
            }
            if depth != 0 { return Err("unterminated block comment".into()); }
            continue;
        }
        // r"...", r#"..."#, br#"..."#, cr#"..."#. r#identifier is not a string.
        let r = if b[i] == b'r' { Some(i) }
            else if matches!(b[i], b'b' | b'c') && b.get(i + 1) == Some(&b'r') { Some(i + 1) }
            else { None };
        if let Some(r) = r {
            let mut open = r + 1; while b.get(open) == Some(&b'#') { open += 1; }
            if b.get(open) == Some(&b'"') {
                let end = format!("\"{}", "#".repeat(open - r - 1));
                let tail = &source[open + 1..];
                let offset = tail.find(&end).ok_or("unterminated raw string")?;
                i = open + 1 + offset + end.len(); continue;
            }
        }
        if b[i] == b'"' {
            i += 1; let mut closed = false;
            while i < b.len() {
                match b[i] {
                    b'\\' => { i += 1; if i < b.len() { i += 1; } },
                    b'"' => { i += 1; closed = true; break; },
                    _ => i += 1,
                }
            }
            if !closed { return Err("unterminated string".into()); }
            continue;
        }
        if b[i] == b'\'' {
            // Skip a single character literal; a lifetime such as 'static remains tokens.
            let mut end = i + 1;
            if b.get(end) == Some(&b'\\') {
                end += 1;
                match b.get(end) {
                    Some(b'u') if b.get(end + 1) == Some(&b'{') => {
                        end += 2; while end < b.len() && b[end] != b'}' { end += 1; }
                        if end < b.len() { end += 1; }
                    }
                    Some(b'x') => end = (end + 3).min(b.len()),
                    Some(_) => end += 1,
                    None => {},
                }
            } else if let Some(ch) = source.get(end..).and_then(|s| s.chars().next()) {
                end += ch.len_utf8();
            }
            if b.get(end) == Some(&b'\'') { i = end + 1; continue; }
        }
        let start = i;
        if b[i].is_ascii_alphabetic() || b[i] == b'_' {
            i += 1;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') { i += 1; }
        } else {
            i += source[i..].chars().next().ok_or("invalid token boundary")?.len_utf8();
        }
        tokens.push(&source[start..i]);
    }
    Ok(tokens)
}

fn c8_sampler_dormancy(source: &str) -> Result<(), String> {
    let tokens = c8_sampler_tokens(source)?;
    let boundary = ["#", "[", "cfg", "(", "test", ")", "]", "mod", "tests", "{"];
    let starts: Vec<_> = tokens.windows(boundary.len()).enumerate()
        .filter_map(|(i, row)| (row == boundary).then_some(i)).collect();
    if starts.len() != 1 { return Err("expected one explicit cfg(test) tests module".into()); }
    let start = starts[0]; let open = start + boundary.len() - 1;
    let mut depth = 0usize; let mut close = None;
    for (i, token) in tokens.iter().enumerate().skip(open) {
        if *token == "{" { depth += 1; }
        if *token == "}" {
            depth = depth.checked_sub(1).ok_or("unbalanced tests module")?;
            if depth == 0 { close = Some(i); break; }
        }
    }
    let close = close.ok_or("unbalanced tests module")?;
    c8_sampler_test_suffix(&tokens[close + 1..])?;
    let production = &tokens[..start];
    if production.iter().any(|token| *token == "mod")
        || production.windows(2).any(|row| row == ["include", "!"]) {
        return Err("new production module/include requires extending dormant-sampler scan".into());
    }
    let references: Vec<_> = production.iter().enumerate()
        .filter_map(|(i, token)| (*token == "checked_submit_sender").then_some(i)).collect();
    if references.len() != 1 {
        return Err(format!("dormant sampler has {} executable identifier references; only its definition is allowed", references.len()));
    }
    let name = references[0];
    if name < 2 || production[name - 1] != "fn" || production.get(name + 1) != Some(&"(") {
        return Err("sampler reference is not its function definition".into());
    }
    // The actual declaration follows an attribute (']'); plain private functions may
    // follow another item ('}' / ';'). pub, pub(crate), async/unsafe/extern widen/change it.
    if !matches!(production[name - 2], "]" | "}" | ";") {
        return Err("sampler must remain a private ordinary function in the scanned module".into());
    }
    Ok(())
}

// Items after the tests module may only be further #[cfg(test)] modules (`mod m;` or `mod m { .. }`, other attributes
// such as #[path] after the cfg) - e.g. A-ID's pty_allocator_busy_tests (R-C8DORM-1 batch, integration merge c00575b6).
// Anything else there would be unscanned production code and fails closed.
fn c8_sampler_test_suffix(tokens: &[&str]) -> Result<(), String> {
    const CFG_TEST: [&str; 7] = ["#", "[", "cfg", "(", "test", ")", "]"];
    let refuse = || Err::<(), String>("only #[cfg(test)] modules may follow the tests module; unscanned suffix".into());
    let mut i = 0;
    while i < tokens.len() {
        if tokens.get(i..i + CFG_TEST.len()) != Some(&CFG_TEST[..]) { return refuse(); }
        i += CFG_TEST.len();
        while tokens.get(i) == Some(&"#") && tokens.get(i + 1) == Some(&"[") {
            let mut depth = 0usize;
            loop {
                match tokens.get(i) {
                    Some(&"[") => depth += 1,
                    Some(&"]") => { depth -= 1; if depth == 0 { break; } }
                    None => return refuse(),
                    _ => {}
                }
                i += 1;
            }
            i += 1;
        }
        let named = matches!(tokens.get(i + 1), Some(t) if t.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_'));
        if tokens.get(i) != Some(&"mod") || !named { return refuse(); }
        i += 2;
        match tokens.get(i) {
            Some(&";") => i += 1,
            Some(&"{") => {
                let mut depth = 0usize;
                loop {
                    match tokens.get(i) {
                        Some(&"{") => depth += 1,
                        Some(&"}") => { depth -= 1; if depth == 0 { break; } }
                        None => return refuse(),
                        _ => {}
                    }
                    i += 1;
                }
                i += 1;
            }
            _ => return refuse(),
        }
    }
    Ok(())
}

#[test]
fn c8_checked_submit_sender_has_no_non_test_reference() {
    c8_sampler_dormancy(include_str!("handlers.rs"))
        .expect("dormant checked_submit_sender acquired a production reference or an unscanned scope");
}

#[test]
fn c8_sampler_dormancy_pin_distinguishes_code_from_literals_and_tests() {
    let allowed = r####"
        fn prior() {}
        // checked_submit_sender(); /* only a comment */
        /* outer /* checked_submit_sender */ still comment */
        const TEXT: &str = "checked_submit_sender(); }";
        const RAW: &[u8] = br###"checked_submit_sender() { }"###;
        const CHAR: char = '}';
        #[allow(dead_code)] fn checked_submit_sender() {}
        #[cfg(test)] mod tests { fn allowed() { checked_submit_sender(); } }
    "####;
    assert!(c8_sampler_dormancy(allowed).is_ok());
    for added in [
        "fn new_handler() { checked_submit_sender (); }",
        "fn new_handler() { checked_submit_sender\n(); }",
        "fn new_handler() { let alias = checked_submit_sender; }",
        "mod unscanned;",
        "include!(\"unscanned.rs\");",
    ] {
        let changed = allowed.replace("#[cfg(test)] mod tests", &format!("{added}\n#[cfg(test)] mod tests"));
        assert!(c8_sampler_dormancy(&changed).is_err(), "pin missed executable reference/scope: {added}");
    }
    let public = allowed.replace("fn checked_submit_sender()", "pub(crate) fn checked_submit_sender()");
    assert!(c8_sampler_dormancy(&public).is_err(), "pin allowed private sampler to widen beyond scanned module");
    let suffix = format!("{allowed}\nfn after_tests() {{ checked_submit_sender(); }}");
    assert!(c8_sampler_dormancy(&suffix).is_err(), "pin missed production suffix after tests module");
    // Further #[cfg(test)] modules may follow the tests module (A-ID's pty_allocator_busy_tests shape); any other
    // item after them, or a non-module / other-cfg item there, still fails closed.
    for trailing in ["#[cfg(test)]\n#[path = \"pty_allocator_busy_tests.rs\"]\nmod pty_allocator_busy_tests;",
                     "#[cfg(test)] mod more { fn t() { checked_submit_sender(); } }\n#[cfg(test)] mod last;"] {
        assert!(c8_sampler_dormancy(&format!("{allowed}\n{trailing}")).is_ok(), "pin refused a trailing test module: {trailing}");
    }
    for trailing in ["#[cfg(test)] #[path = \"x.rs\"] mod x;\nfn after() {}",
                     "#[cfg(test)] mod x;\nfn after() { checked_submit_sender(); }",
                     "#[cfg(test)] mod a;\nfn hidden() { checked_submit_sender(); }\n#[cfg(test)] mod b;", // R-C8DORM-2 #2
                     "#[cfg(test)] fn helper() {}",
                     "#[cfg(not(test))] mod z;",
                     "#[cfg(test)] mod open {"] {
        assert_eq!(c8_sampler_dormancy(&format!("{allowed}\n{trailing}")),
            Err("only #[cfg(test)] modules may follow the tests module; unscanned suffix".to_string()),
            "pin accepted a non-test suffix item: {trailing}");
    }
}
