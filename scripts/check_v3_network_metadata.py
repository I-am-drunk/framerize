#!/usr/bin/env python3
"""Compare the remaining network assignments and their ordered composition."""
import json
import os
from pathlib import Path
import runpy
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
shared = runpy.run_path(str(ROOT/'scripts/check_v3_network_hash.py'))
array, string, number, utf16 = [shared[n] for n in ('array','string','number','utf16')]
UNDEFINED, NULL = shared['UNDEFINED'], shared['NULL']
FIELDS = [('related',40,80,'NetworkRelatedAddress'),
          ('protocols',18,659,'NetworkProtocols')]


def network(fields):
    row = [UNDEFINED]*56
    for index, value in fields.items():
        row[index] = value
    return array([array(row)])


def full_cases():
    values = {18:'udp+tcp',38:'192.0.2.1',40:'198.51.100.2',47:'0123abcd'}
    base = {index:string(utf16(value)) for index,value in values.items()}
    result = [('complete',network(base)), ('empty',network({})),
              ('outer_null',NULL), ('missing_row',array([])),
              ('null_row',array([NULL]))]
    object_value = shared['object_value']
    result.append(('object_record', object_value([('0', object_value(
        [(str(index), value) for index, value in base.items()]))])))
    for index in (40,47,38,18):
        result.append((f'array_field_{index}',network({**base,index:array([number(1)])})))
        result.append((f'empty_field_{index}',network({**base,index:string([])})))
    return result


def check():
    selected = [(kind,position,variant,name,value)
                for kind,index,position,variant in FIELDS
                for name,value in shared['cases'](index)]
    full = full_cases()
    inputs = [dict(kind=k,compact=True,value=v[0]) for k,_,_,_,v in selected]
    inputs += [dict(kind='network_all',value=v[0]) for _,v in full]
    result = subprocess.run([os.environ.get('NODE','node'),
        str(ROOT/'scripts/castle_v3_collector_reference.mjs')],
        input=json.dumps(inputs),capture_output=True,text=True,check=True,timeout=10)
    expected = json.loads(result.stdout)
    assert len(expected) == len(inputs)
    rows = []
    for (_,position,variant,name,(_,expr)),outcome in zip(selected,expected):
        assert outcome.get('error') in (None,'TypeError'), name
        data = 'None' if 'error' in outcome else f'Some(&{outcome["bytes"]})'
        rows.append(f'({json.dumps(name)},{position},CollectorResult::{variant}({expr}),{data})')
    module = json.dumps(str(ROOT/'crates/framer-native/src/castle/mod.rs'))
    source = f'''#[allow(dead_code,unused_imports)] #[path={module}] mod castle;
use castle::v3::*;
fn base() -> Vec<Value> {{ (1000..1765).map(|n|Value::Number(n as f64)).collect() }}
#[test] fn selected_metadata_assignments() {{
    let cases:[(&str,usize,CollectorResult,Option<&[u8]>);{len(rows)}]=[{','.join(rows)}];
    for (name,position,input,expected) in cases {{
        let mut values=base(); let before=serialize(&values).unwrap();
        let result=apply_collector_result(&mut values,input);
        if let Some(bytes)=expected {{ result.unwrap();
            let value=std::mem::replace(&mut values[position],Value::Number((1000+position) as f64));
            assert_eq!(serialize(&[value]).unwrap(),bytes,"{{name}}");
        }} else {{ assert_eq!(result,Err(CollectorError::Network(NetworkValueError::MissingRecord))); }}
        assert_eq!(serialize(&values).unwrap(),before,"{{name}}");
    }}
}}
'''
    for i,((name,(_,expr)),outcome) in enumerate(zip(full,expected[len(selected):])):
        assert outcome.get('error') in (None,'TypeError'), name
        ok = str('error' not in outcome).lower()
        source += f'''#[test] fn ordered_callback_{i}() {{
            let mut values=base();
            let result=apply_network_result(&mut values,{expr});
            assert_eq!(result.is_ok(),{ok},"{name}");
            assert_eq!(serialize(&values).unwrap().as_slice(),&{outcome['bytes']},"{name}");
        }}
'''
    source += refusal_tests()
    with tempfile.TemporaryDirectory(prefix='v3-network-metadata-') as directory:
        path = Path(directory)
        (path/'check.rs').write_text(source)
        compile_fixture = runpy.run_path(str(ROOT/'scripts/v3_native_fixture.py'))['compile_fixture']
        compile_fixture(path/'check.rs',path/'check')
        subprocess.run([str(path/'check'),'--test-threads=1'],check=True,timeout=10)
    print(f'PASS: {len(selected)} metadata comparisons and {len(full)} complete callback comparisons')


def refusal_tests():
    return '''
fn record(index:usize,value:Value)->Value {
    let mut row:Vec<_>=(0..56).map(|_|Value::Undefined).collect();
    row[index]=value; Value::Array(vec![Value::Array(row)])
}
#[test] fn ordered_assignment_refuses_growth_and_retains_the_completed_prefix() {
    for missing in [80,469,473,659] {
        let mut expected=base();
        apply_network_result(&mut expected,Value::Null).unwrap();
        expected.truncate(missing);
        let mut values=base(); values.truncate(missing);
        assert_eq!(apply_network_result(&mut values,Value::Null),
            Err(CollectorError::MissingPosition(missing)));
        assert_eq!(serialize(&values).unwrap(),serialize(&expected).unwrap());
    }
    let mut values=base(); let before=serialize(&values).unwrap();
    assert_eq!(apply_network_result(&mut values,Value::JsonFailure),
        Err(CollectorError::Network(NetworkValueError::NormalizedValue)));
    assert_eq!(serialize(&values).unwrap(),before);
}
#[test] fn unsupported_fields_and_missing_positions_preserve_input() {
    let variants:[(fn(Value)->CollectorResult,usize,usize);2]=[
        (CollectorResult::NetworkRelatedAddress,80,40),
        (CollectorResult::NetworkProtocols,659,18)];
    for (variant,position,field) in variants {
        for input in [Value::JsonFailure,record(field,Value::JsonFailure),
            record(field,Value::JsonFallback(vec![48])),record(field,Value::Number(1.0)),
            record(field,Value::Array(vec![])),record(field,Value::Object(vec![]))] {
            let mut values=base(); let before=serialize(&values).unwrap();
            assert!(apply_collector_result(&mut values,variant(input)).is_err());
            assert_eq!(serialize(&values).unwrap(),before);
        }
        let mut values=base(); values.truncate(position);
        let before=serialize(&values).unwrap();
        assert_eq!(apply_collector_result(&mut values,variant(Value::Null)),
            Err(CollectorError::MissingPosition(position)));
        assert_eq!(serialize(&values).unwrap(),before);
    }
}
'''


if __name__ == '__main__':
    check()
