use landlock::{ABI, Ruleset, RulesetAttr, AccessNet, AccessFs};

fn main() {
    let best_abi = landlock::ABI::V4;
    println!("Testing {:?}", best_abi);
}
