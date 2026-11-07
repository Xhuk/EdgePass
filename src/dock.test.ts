import { describe, expect, it } from "vitest";
import {
  FLOAT_COLLAPSED,
  floatCollapsedSize,
  floatCollapsedWidth,
  nearestEdge,
  placementFrom,
} from "./dock";

const WORK = { x: 0, y: 0, w: 1000, h: 800 };

describe("floatCollapsedWidth", () => {
  it("keeps the base width through three LEDs", () => {
    expect(floatCollapsedWidth(0)).toBe(FLOAT_COLLAPSED.w);
    expect(floatCollapsedWidth(3)).toBe(FLOAT_COLLAPSED.w);
  });

  it("grows 50 px per extra LED", () => {
    expect(floatCollapsedWidth(4)).toBe(FLOAT_COLLAPSED.w + 50);
    expect(floatCollapsedWidth(8)).toBe(FLOAT_COLLAPSED.w + 5 * 50);
  });

  it("only grows the width, never the height", () => {
    expect(floatCollapsedSize(8).h).toBe(FLOAT_COLLAPSED.h);
  });
});

describe("nearestEdge", () => {
  it("snaps to the closest side", () => {
    expect(nearestEdge(10, 300, 200, 56, WORK)).toBe("left");
    expect(nearestEdge(790, 300, 200, 56, WORK)).toBe("right");
  });

  it("prefers a side when the top is equally close", () => {
    expect(nearestEdge(5, 5, 200, 56, WORK)).toBe("left");
  });

  it("does not snap a window in the middle", () => {
    expect(nearestEdge(400, 400, 200, 56, WORK)).toBeNull();
  });
});

describe("placementFrom", () => {
  it("centres the floating pill on the width it was given", () => {
    const p = placementFrom({ x: 100, y: 100, w: 660, h: 56 }, WORK, null, "", 660);
    expect(p.x).toBeCloseTo((100 + 330) / 1000, 6);
    expect(p.dock).toBeNull();
  });
});
