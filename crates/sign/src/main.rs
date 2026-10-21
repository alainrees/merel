//! `merel-sign keygen` makes the release key; `merel-sign sign <file>` writes `<file>.sig`
//! with the secret key from the `MEREL_SIGNING_KEY` environment variable.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["keygen"] => match merel_sign::keygen() {
            Ok((secret, public)) => {
                println!(
                    "Geheime sleutel. Zet die in GitHub als secret MEREL_SIGNING_KEY en bewaar hem ook in je wachtwoordbeheerder (GitHub laat hem niet meer zien). Nooit in de repo of in een chat:\n\n{}\n",
                    *secret
                );
                println!("Publieke sleutel. Die komt in de app:\n\n{public}");
                ExitCode::SUCCESS
            }
            Err(e) => fail(&e.to_string()),
        },
        ["sign", file] => {
            let Ok(secret) = std::env::var("MEREL_SIGNING_KEY") else { return fail("MEREL_SIGNING_KEY is not set") };
            let message = match std::fs::read(file) {
                Ok(m) => m,
                Err(e) => return fail(&format!("{file}: {e}")),
            };
            let signature = match merel_sign::sign(&secret, &message) {
                Ok(s) => s,
                Err(e) => return fail(&e.to_string()),
            };
            match std::fs::write(format!("{file}.sig"), format!("{signature}\n")) {
                Ok(()) => {
                    // The public key in the log, so a wrong secret shows up at once.
                    println!("signed {file}; public key {}", merel_sign::public_key(&secret).unwrap_or_default());
                    ExitCode::SUCCESS
                }
                Err(e) => fail(&format!("{file}.sig: {e}")),
            }
        }
        _ => fail("usage: merel-sign keygen | merel-sign sign <file>"),
    }
}

fn fail(message: &str) -> ExitCode {
    eprintln!("merel-sign: {message}");
    ExitCode::FAILURE
}
