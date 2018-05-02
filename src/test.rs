#![cfg(test)]

use crate::intern;
use crate::output::{Algorithm, Output};
use crate::tab_delim;
use failure::Error;
use std::path::Path;

fn test_fn(dir_name: &str, fn_name: &str) -> Result<(), Error> {
    do catch {
        let facts_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("inputs")
            .join(dir_name)
            .join("nll-facts")
            .join(fn_name);
        println!("facts_dir = {:?}", facts_dir);
        let tables = &mut intern::InternerTables::new();
        let all_facts = tab_delim::load_tab_delimited_facts(tables, &facts_dir)?;
        let _result = Output::compute(all_facts, Algorithm::Naive, false);
        // FIXME: check `_result` somehow
    }
}

macro_rules! tests {
    ($($name:ident($dir:expr, $fn:expr),)*) => {
        $(
            #[test]
            fn $name() -> Result<(), Error> {
                test_fn($dir, $fn)
            }
        )*
    }
}

tests! {
    issue_47680("issue-47680", "main"),
}
