use crate::{
    config::Explain as ExplainConfig,
    err::{ExplainErr, StatixErr},
};

use lib::LINTS;

pub fn explain(code: u32) -> Result<&'static str, ExplainErr> {
    match code {
        0 => Ok("syntax error"),
        _ => LINTS
            .iter()
            .find(|l| l.code() == code)
            .map(|l| l.explanation())
            .ok_or(ExplainErr::LintNotFound(code)),
    }
}

pub fn run(explain_config: &ExplainConfig) -> Result<(), StatixErr> {
    let explanation = explain(explain_config.target)?;
    println!("{explanation}");
    Ok(())
}
