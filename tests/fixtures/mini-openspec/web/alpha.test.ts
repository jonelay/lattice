// A Jest test: readable by neither dialect the adapter knows.
test("alpha parses", () => {
  expect(parse("x")).toBeTruthy();
});
