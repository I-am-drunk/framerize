#!/usr/bin/env python3
"""Re-encode an isolated SDK's normalized wire snapshot without logging it."""
import base64
import hashlib
import json
from pathlib import Path
import runpy
import struct
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]


class Reader:
    def __init__(self, data):
        self.data, self.at, self.values = data, 0, 0

    def read(self, size):
        end = self.at + size
        if end > len(self.data):
            raise ValueError('truncated snapshot')
        value, self.at = self.data[self.at:end], end
        return value

    def unsigned(self):
        value = 0
        for shift in range(0, 35, 7):
            byte = self.read(1)[0]
            value |= (byte & 127) << shift
            if not byte & 128 and value <= 0xffffffff:
                return value
        raise ValueError('invalid snapshot integer')

    def array(self, depth=0):
        count = self.unsigned()
        if depth > 128 or count > 50000:
            raise ValueError('snapshot limit')
        return [self.value(depth+1) for _ in range(count)]

    def value(self, depth):
        self.values += 1
        if self.values > 50000:
            raise ValueError('snapshot limit')
        tag = self.read(1)[0]
        if tag == 0:
            return 'Value::Undefined'
        if tag in (5, 6):
            return 'Value::Bool('+str(tag == 6).lower()+')'
        if tag in (3, 10):
            n = int.from_bytes(self.read(4),'little') if tag == 3 else self.unsigned()
            n = n if tag == 3 else (n >> 1) ^ -(n & 1)
            return f'Value::Number({n}.0)'
        if tag == 4:
            bits = int.from_bytes(self.read(8),'big')
            return f'Value::Number(f64::from_bits({bits}))'
        if tag == 7:
            return 'Value::Array(vec!['+','.join(self.array(depth))+'])'
        if tag not in (1, 2, 8, 9):
            raise ValueError('unknown snapshot type')
        count = self.unsigned()
        if tag in (1, 8):
            units = [value for (value,) in struct.iter_unpack('<H',self.read(count*2))]
        else:
            data = self.read(count)
            units = list(base64.b64encode(data) if tag == 2 else data)
        kind = 'JsonFallback' if tag == 8 else 'String'
        return f'Value::{kind}(vec!{units})'


def check(snapshot, output):
    raw = snapshot.read_bytes()
    if len(raw) > 8*1024*1024:
        raise ValueError('snapshot size')
    data = json.loads(raw)
    assert data['schema'] == 1 and data['synthetic'] is True
    assert data['sdk_sha256'] == '3337eba7ffe614c2ebd9b2b8a55794f2e2445a9b3658d1af5eb7530152b72c71'
    assert all(type(x) is int and 0 <= x <= 255 for x in data['serialized'])
    serialized = bytes(data['serialized'])
    reader = Reader(serialized)
    values = reader.array()
    assert reader.at == len(serialized)
    timestamp = data['timestamp_ms']
    assert type(timestamp) is int and abs(timestamp) <= 8640000000000000
    with tempfile.TemporaryDirectory(prefix='v3-sdk-snapshot-') as directory:
        path = Path(directory)
        (path/'serialized').write_bytes(serialized)
        module = json.dumps(str(ROOT/'crates/framer-native/src/castle/mod.rs'))
        source = f'''#![recursion_limit="2048"]
#[allow(dead_code,unused_imports)] #[path={module}] mod castle;
use castle::v3::{{Value,serialize,encode_collected}};
#[test] fn normalized_snapshot_round_trip_and_encode() {{
    let values=vec![{','.join(values)}];
    let expected=include_bytes!({json.dumps(str(path/'serialized'))});
    assert!(serialize(&values).unwrap()==expected,"snapshot representation differs");
    let token=encode_collected(&values,{timestamp}).unwrap();
    std::fs::write({json.dumps(str(path/'native-token'))},token).unwrap();
}}
'''
        (path/'check.rs').write_text(source)
        compile_fixture = runpy.run_path(str(ROOT/'scripts/v3_native_fixture.py'))['compile_fixture']
        compile_fixture(path/'check.rs',path/'check')
        subprocess.run([str(path/'check'),'--test-threads=1'],check=True,timeout=20)
        token = (path/'native-token').read_bytes()
        equal = hashlib.sha256(token).hexdigest() == data['token_sha256']
        equal = equal and len(token) == data['token_bytes']
    report = dict(schema=1,synthetic=True,normalized_snapshot_round_trip=True,
                  token_sha256_equal=equal,positions=len(values),values=reader.values,
                  serialized_bytes=len(serialized),token_bytes=data['token_bytes'],
                  snapshot_sha256=hashlib.sha256(raw).hexdigest())
    output.write_text(json.dumps(report,indent=2)+'\n')
    if not equal:
        raise ValueError('native encoding differs from the instrumented SDK')
    print('PASS: actual SDK normalized snapshot and native token encoding agree')


if __name__ == '__main__':
    check(Path(sys.argv[1]),Path(sys.argv[2]))
