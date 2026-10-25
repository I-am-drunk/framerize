#!/usr/bin/env python3
"""Compare native RTC normalization with the pinned source, without network I/O."""
import json
import os
from pathlib import Path
import runpy
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
shared = runpy.run_path(str(ROOT / 'scripts/check_v3_objects.py'))
utf16, number = shared['utf16'], shared['number']


def candidate(**values):
    return {'kind': 'candidate', 'value': values}


def cases():
    result = [('empty', [])]
    base = dict(type='HOST', protocol='UDP', address='192.0.2.1', port='50000', component='rtp')
    for name, changes in [('ordinary', {}), ('tcp', dict(protocol='TCP', tcpType='PASSIVE')),
                          ('ipv6', dict(address='2001:db8::1', relatedAddress='host.local')),
                          ('unicode', dict(type='ΟΣ', protocol='İ', component='\ud800'))]:
        result.append((name, [candidate(**(base | changes))]))
    for key in ['type', 'protocol', 'address', 'relatedAddress', 'tcpType', 'component']:
        result.append(('only_'+key, [candidate(**{key: 'VALUE'})]))
    for port in ['-1', '0', '0.5', '1023.99', '1024', '49151.9', '49152', '65536', 'NaN', 'Infinity']:
        result.append(('port_'+port, [candidate(**(base | dict(port=port)))]))
    raw = 'candidate:1 1 UDP 99 192.0.2.1 50000 typ HOST'
    strings = [raw, raw.upper(), raw+' raddr 2001:db8::1 tcptype PASSIVE',
               raw+' tcptype ACTIVE raddr foo.local', raw+' raddr a raddr b tcptype so',
               raw+' raddr a tcptype so tcptype passive', raw+'\nraddr host.local',
               raw+'\nextra raddr host.local', raw+'\r\nraddr host.local',
               raw+' raddr\nfoo.local tcptype active', raw+' raddr',
               raw.replace(' ', '\u00a0'), raw.replace(' ', '\u0085'),
               raw.replace('1 1 ', '1 +1 '), raw.replace('50000', '0'),
               raw.replace('50000', '9'*400), raw.replace('50000', '1.5'),
               raw.replace('typ', 'type'), raw.replace('candidate:1', 'candidate:'),
               raw.replace('candidate:1', 'x-candidate:1'), raw+' tail', '', 'malformed']
    for i, value in enumerate(strings):
        result.append((f'candidate_text_{i}', [candidate(candidate=value)]))
    result.append(('structured_wins', [candidate(type='HOST', candidate='malformed')]))
    result.append(('empty_uses_text', [candidate(type='', protocol='', address='', candidate=raw)]))
    result.append(('signature_collision', [candidate(type='a:b', protocol='c'),
                                           candidate(type='a', protocol='b:c')]))
    rows = [candidate(**(base | dict(component=str(i)))) for i in range(20)]
    offer = {'kind': 'sdp', 'value': 'UDP/TLS/RTP/SAVPF DTLS/SCTP UDP'}
    result += [('cap_after_dedup', [rows[0]]*20+rows), ('sdp_first', [offer]+rows),
               ('sdp_last', rows+[offer]), ('sdp_at_cap', rows[:15]+[offer])]
    for i, value in enumerate(['xUDP UDPx _UDP UDP_ 1UDP UDP1',
                                'éUDP-éDTLS/SCTP UDP/TLS/RTP/SAVPFx',
                                'udp dtls/sctp UDP/TLS/RTP/SAVPF', 'nothing']):
        result.append((f'sdp_words_{i}', [{'kind': 'sdp', 'value': value}]))
    values = [None, False, 3, '', 'NEW', 'new', 'Connected', 'ΟΣ', 'ΟΣΑ',
              'İ', 'I', '\ud800A', '😀', 'closed', 'late']
    result.append(('state_reduction', [{'kind': 'state', 'group': group, 'value': value}
                  for group in range(3) for value in values]))
    return result


def float_code(value):
    return number(float(value))[1][len('Value::Number('):-1]


def event_code(event):
    if event['kind'] == 'candidate':
        names = {'type': 'kind', 'relatedAddress': 'related_address', 'tcpType': 'tcp_type'}
        fields = []
        for key, value in event['value'].items():
            code = 'None' if value is None else ('Some('+float_code(value)+')' if key == 'port'
                    else f'Some(vec!{utf16(value)})')
            fields.append(f'{names.get(key,key)}:{code}')
        return 'events.observe_candidate(&CandidateObservation{'+','.join(fields)+',..Default::default()});'
    if event['kind'] == 'sdp':
        return f'events.observe_sdp(&{utf16(event["value"])});'
    group = ['Connection', 'Gathering', 'Signaling'][event['group']]
    value = event['value']
    if isinstance(value, str):
        code = f'Value::String(vec!{utf16(value)})'
    elif value is None:
        code = 'Value::Null'
    elif isinstance(value, bool):
        code = 'Value::Bool('+str(value).lower()+')'
    else:
        code = number(float(value))[1]
    return f'events.observe_state(NetworkState::{group}, &{code});'


def check():
    selected = cases()
    addresses = ['', '1.2.3.4', '999.000.1.9999', '1.2.3', '1.2.3.4.5', ':',
                 'host.local', 'A-Z9', '_host', 'host/name', 'é', '\ud800', 'x:y', ' ']
    addresses += [value+end for value in ['1.2.3.4', 'host', ':']
                  for end in ['\n', '\r', '\r\n', '\u2028', '\u2029', '\n\n']]
    ports = ['-Infinity', '-1', '-0', '0', '0.5', '1023.9', '1024', '49151.9', '49152', '65535', '1e300', 'Infinity', 'NaN']
    data = dict(addresses=addresses, ports=ports, cases=[events for _, events in selected])
    reference = subprocess.run([os.environ.get('NODE', 'node'), str(ROOT/'scripts/castle_v3_network_events_reference.mjs')],
        input=json.dumps(data), text=True, capture_output=True, timeout=10)
    if reference.returncode:
        raise RuntimeError(reference.stderr)
    expected = json.loads(reference.stdout)
    assert len(expected['observations']) == len(selected)
    tests = []
    for index, ((name, events), result) in enumerate(zip(selected, expected['observations'])):
        statements = '\n'.join(event_code(event) for event in events)
        order = ','.join(f'vec!{units}' for units in result['order'])
        assert all(type(b) is int and 0 <= b <= 255 for b in result['bytes'])
        tests.append(f'''#[test] fn events_{index}_{name.replace('-', '_').replace('.', '_')}() {{
            let mut events=NetworkEvents::default();
            {statements}
            let observation=events.into_observation(vec!{utf16('192.0.2.1')},vec![]);
            let order: Vec<Vec<u16>>=vec![{order}];
            let value=assemble_network_record(observation,|a,b| {{
                let rank=|v:&[u16]| order.iter().position(|p|p==v).expect("selected signature");
                rank(a).cmp(&rank(b))
            }}).unwrap();
            assert_eq!(serialize(&[value]).unwrap(),vec!{result['bytes']},"{name}");
        }}''')
    checks = [f'assert_eq!(network_address_class(&{utf16(v)}),{json.dumps(e)},"address {i}");'
              for i, (v,e) in enumerate(zip(addresses, expected['classes']))]
    checks += [f'assert_eq!(network_port_class({float_code(v)}),{json.dumps(e)},"port {i}");'
               for i,(v,e) in enumerate(zip(ports, expected['ports']))]
    tests.append('#[test] fn classifications() {'+'\n'.join(checks)+'}')
    module=json.dumps(str(ROOT/'crates/framer-native/src/castle/mod.rs'))
    source=f'#![recursion_limit="1024"]\n#[allow(dead_code)] #[path={module}] mod castle;\nuse castle::v3::*;\n'+'\n'.join(tests)
    with tempfile.TemporaryDirectory(prefix='castle-v3-events-') as directory:
        path=Path(directory)
        (path/'check.rs').write_text(source)
        compile_fixture=runpy.run_path(str(ROOT/'scripts/v3_native_fixture.py'))['compile_fixture']
        compile_fixture(path/'check.rs',path/'check')
        subprocess.run([str(path/'check'),'--test-threads=1'],check=True,timeout=10)
    print(f'PASS: {len(selected)} ordered event comparisons, {len(addresses)} address and {len(ports)} port comparisons')
    print(f'Reference Unicode version: {expected["unicode"]}; event collection and locale collation remain caller-owned')


if __name__ == '__main__':
    check()
