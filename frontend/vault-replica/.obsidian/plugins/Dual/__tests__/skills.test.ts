import { getLastBlock } from "../skills";

describe("Skills", () => {
  describe("`getLastBlock`", () => {
    test("gets last non-empy block", () => {
      expect(getLastBlock(["this", "is", "a", "block", ""])).toBe("block");
    });
    test("return an empy block when that's all there is", () => {
      expect(getLastBlock(["", "", "", ""])).toBe("");
    });
  });
});
