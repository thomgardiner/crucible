// Tiny pure function used by the demo's mutation capture (shouldBuy).
export function shouldBuy(price: number, cap: number): boolean {
  return price <= cap;
}

// scope-pin 61896
