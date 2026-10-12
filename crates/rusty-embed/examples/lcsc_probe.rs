//! What EasyEDA's component service actually answers for an LCSC part, and
//! what `schematic::easyeda` makes of it — the way to capture a fixture and
//! the way to see why a symbol came out wrong.
//!
//!     cargo run -p rusty-embed --example lcsc_probe -- C25804 [out.json]
//!
//! Prints the part's title and parameters, the anchor, every `shape` record
//! raw, and then the symbol as read. With a second argument the raw answer
//! is written to that file, which is what the tests' fixtures are.

use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(number) = args.next() else {
        eprintln!("usage: lcsc_probe <C-number> [out.json]");
        return ExitCode::from(2);
    };
    let out = args.next();
    let number = match rusty_embed::schematic::easyeda::part_number(&number) {
        Ok(number) => number,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(2);
        }
    };
    let body = match rusty_embed::schematic::easyeda::fetch(&number) {
        Ok(body) => body,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(1);
        }
    };
    if let Some(out) = &out {
        if let Err(error) = std::fs::write(out, &body) {
            eprintln!("could not write {out}: {error}");
        } else {
            println!("raw answer written to {out} ({} bytes)", body.len());
        }
    }

    let value: serde_json::Value = match serde_json::from_str(&body) {
        Ok(value) => value,
        Err(error) => {
            eprintln!(
                "not JSON ({error}); the first bytes were:\n{}",
                &body[..body.len().min(400)]
            );
            return ExitCode::from(1);
        }
    };
    let result = &value["result"];
    println!("title: {}", result["title"].as_str().unwrap_or("-"));
    println!(
        "description: {}",
        result["description"].as_str().unwrap_or("-")
    );
    let data = match &result["dataStr"] {
        serde_json::Value::String(text) => serde_json::from_str(text).unwrap_or_default(),
        other => other.clone(),
    };
    println!("head: x={} y={}", data["head"]["x"], data["head"]["y"]);
    if let Some(para) = data["head"]["c_para"].as_object() {
        for (key, value) in para {
            println!("  c_para.{key} = {value}");
        }
    }
    if let Some(shapes) = data["shape"].as_array() {
        println!("{} shape records:", shapes.len());
        for shape in shapes {
            println!("  {}", shape.as_str().unwrap_or("?"));
        }
    }

    match rusty_embed::schematic::easyeda::parse(&number, &body) {
        Ok(imported) => {
            let s = &imported.symbol;
            println!(
                "\nsymbol {}: reference {} value {:?} description {:?}",
                s.id(),
                s.reference,
                s.value,
                s.description
            );
            for pin in &s.pins {
                println!("  pin {:?}", pin);
            }
            for graphic in &s.graphics {
                println!("  {:?}", graphic);
            }
            for warning in &imported.warnings {
                println!("  warning: {warning}");
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(1)
        }
    }
}
