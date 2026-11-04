export function capture(value) {
  if (value === undefined) return { kind: 'undefined' };
  if (value === null) return { kind: 'null' };
  if (typeof value === 'boolean') return { kind: 'boolean', value };
  if (typeof value === 'number') return {
    kind: 'number', value: Object.is(value, -0) ? '-0' : String(value),
  };
  if (typeof value === 'string') return {
    kind: 'string', units: Array.from({ length: value.length }, (_, index) => value.charCodeAt(index)),
  };
  return { kind: 'array', values: value.map(capture) };
}

export function frame(text) {
  const body = Buffer.from(text);
  const header = Buffer.alloc(12);
  header.write('FCLT');
  header[4] = 1;
  header.writeUInt32BE(body.length, 8);
  return Buffer.concat([header, body]);
}

export function collectionInput(values, worker, network) {
  return {
    version: 3, values: capture(values),
    shared_worker_reply: capture(worker), network_reply: capture(network),
  };
}

export function* collectionInputs() {
  const markers = ['83au4jz', '3mxkmwkle0', '9q5p9p15pn', '99j32wg9x',
    'gp9vlcc', '2v315vt89', 'vrbhklehy', 'ydlkmari0n'];
  for (let index = 0; index < 128; index += 1) {
    const values = Array(765).fill(undefined);
    values[0] = [undefined, null, -0, NaN, Infinity, -Infinity, '\ud800', true, index];
    values[755] = 'sdk-owned-timing';
    for (const position of [80, 242, 469, 473, 659]) values[position] = 'ignored';
    let worker = markers[index % 12];
    if (index % 12 === 8 || index % 12 === 10) {
      worker = Array(11).fill(undefined);
      worker[0] = 2;
      worker[10] = index % 12 === 8 ? `version-${index}` : false;
    } else if (index % 12 === 9) worker = [1];
    else if (index % 12 === 11) worker = 'custom-string';
    let network;
    if (index % 4 === 1) network = 'observed-non-record';
    if (index % 4 >= 2) {
      const record = Array(48).fill(undefined);
      record[40] = index % 4 === 2 ? '' : `related-${index}`;
      record[47] = `hash-${index}`;
      record[38] = index % 4 === 2 ? `192.0.2.${index}` : [index, true];
      record[18] = index % 4 === 2 ? 'udp+tcp' : '';
      network = [record];
    }
    yield { text: JSON.stringify(collectionInput(values, worker, network)),
      timestamp: 1700000000000 + index };
  }
}
