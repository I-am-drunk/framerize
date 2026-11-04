export function capture(value) {
  if (value === void 0) return { kind: "undefined" };
  if (value === null) return { kind: "null" };
  if (typeof value === "boolean") return { kind: "boolean", value };
  if (typeof value === "number") return { kind: "number", value: Object.is(value, -0) ? "-0" : String(value) };
  if (typeof value === "string") return {
    kind: "string",
    units: Array.from({ length: value.length }, (_, index) => value.charCodeAt(index))
  };
  return { kind: "array", values: value.map(capture) };
}
export function collectionInput(values = Array(765).fill(void 0), worker = void 0, network = void 0) {
  return {
    version: 3,
    values: capture(values),
    shared_worker_reply: capture(worker),
    network_reply: capture(network)
  };
}
export function frame(text) {
  const body = Buffer.from(text);
  const header = Buffer.alloc(12);
  header.write("FCLT");
  header[4] = 1;
  header.writeUInt32BE(body.length, 8);
  return Buffer.concat([header, body]);
}
export function* collectionInputs() {
  const markers = [
    "83au4jz",
    "3mxkmwkle0",
    "9q5p9p15pn",
    "99j32wg9x",
    "gp9vlcc",
    "2v315vt89",
    "vrbhklehy",
    "ydlkmari0n"
  ];
  for (let index = 0; index < 128; index += 1) {
    const values = Array(765).fill(void 0);
    values[0] = [void 0, null, -0, NaN, Infinity, -Infinity, "\uD800", true, index];
    values[755] = "sdk-owned-timing";
    for (const position of [80, 242, 469, 473, 659]) values[position] = "ignored";
    let worker = markers[index % 12];
    if (index % 12 === 8 || index % 12 === 10) {
      const reply = Array(11).fill(void 0);
      reply[0] = 2;
      reply[10] = index % 12 === 8 ? `version-${index}` : false;
      worker = reply;
    } else if (index % 12 === 9) worker = [1];
    else if (index % 12 === 11) worker = "custom-string";
    yield {
      text: JSON.stringify(collectionInput(values, worker, networkInput(index))),
      timestamp: 17e11 + index
    };
  }
}
function networkInput(index) {
  if (index % 4 === 0) return void 0;
  if (index % 4 === 1) return "observed-non-record";
  const record = Array(48).fill(void 0);
  record[40] = index % 4 === 2 ? "" : `related-${index}`;
  record[47] = `hash-${index}`;
  record[38] = index % 4 === 2 ? `192.0.2.${index}` : [index, true];
  record[18] = index % 4 === 2 ? "udp+tcp" : "";
  return [record];
}
