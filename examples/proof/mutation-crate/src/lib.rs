/// Buy only when the price is at or under the cap. The `<=` is load-bearing:
/// at price == cap we must still buy.
pub fn should_buy(price: u32, cap: u32) -> bool {
    price <= cap
}

#[cfg(test)]
mod tests {
    use super::*;

    // Reward-hacked: green, but it never exercises the boundary, so replacing the
    // body with `true` still passes. cargo-mutants reports this as a MISSED mutant.
    #[test]
    fn buys_when_cheap() {
        assert!(should_buy(5, 10));
    }
}
