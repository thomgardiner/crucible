// A real test with a real assertion (clean: crucible test-smells passes it).
it('buys under the cap', () => { expect(shouldBuy(5, 10)).toBe(true); });
it('rejects over the cap', () => { expect(shouldBuy(11, 10)).toBe(false); });
