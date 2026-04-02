use crate::{
    config::Explain as ExplainConfig,
    err::{ExplainErr, StatixErr},
    utils,
};

pub fn explain(code: u32) -> Result<&'static str, ExplainErr> {
    let lints = utils::lint_map();
    match code {
        0 => Ok("syntax error"),
        _ => lints
            .values()
            .flatten()
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
