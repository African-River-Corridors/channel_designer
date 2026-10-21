// The parity comparison fails on a doctored value and passes within the ADR 0005 tolerance.
// Run: node --test tools/web_parity_compare.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { compare } from "./web_parity_compare.mjs";

const doc = { stages: { volumes: { results: { dredge_m3: 416884.123456, faces: [16.2, 3.1] } } }, name: "river" };
const copy = () => structuredClone(doc);

test("equal documents have no problems", () => {
  const r = compare(doc, copy());
  assert.deepEqual(r.problems, []);
  assert.equal(r.numbers, 3);
});

test("a difference inside 1e-9 relative is accepted", () => {
  const b = copy();
  b.stages.volumes.results.dredge_m3 *= 1 + 5e-10;
  assert.deepEqual(compare(doc, b).problems, []);
});

test("a doctored number is a problem, and says where", () => {
  const b = copy();
  b.stages.volumes.results.faces[1] = 3.2;
  const r = compare(doc, b);
  assert.equal(r.problems.length, 1);
  assert.match(r.problems[0], /faces\[1\]/);
});

test("a changed string, a missing key and a different length are problems", () => {
  const b = copy();
  b.name = "other";
  delete b.stages.volumes.results.dredge_m3;
  b.stages.volumes.results.faces.push(1);
  assert.equal(compare(doc, b).problems.length, 3);
});
