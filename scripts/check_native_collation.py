import os
import runpy
import tempfile
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
LOCALES = ['en-US', 'en-GB', 'de', 'fr', 'fr-CA', 'sv', 'da', 'tr', 'pl',
           'cs', 'ja', 'zh', 'th', 'ar', 'es-u-co-trad', 'de-u-co-phonebk',
           'en-US-u-kn-true', 'en-US-u-kf-upper']
TEXT = ['', 'a', 'A', 'b', 'B', 'á', 'a\u0301', 'ä', 'a\u0308', 'å', 'Å',
        'æ', 'Æ', 'z', 'Z', 'ö', 'ø', 'œ', 'ß', 'ss', 'İ', 'I', 'ı', 'i',
        'é', 'e', 'e\u0301', 'ê', 'è', 'ñ', 'n', 'ch', 'c', 'h', 'll',
        '-', '_', ':', '/', '.', '+', ' ', '!', '\x00', 'a\x00', 'a-b',
        'ab', 'a_b', 'a:b', 'a/b', 'a.b', 'a b', '01', '1', '2', '10',
        '100', '٢', '２', 'Ⅳ', '④', 'あ', 'ア', 'ｱ', 'は', 'ば', '中',
        '国', '國', 'ก', 'ข', 'ا', 'ب', 'α', 'ω', 'а', 'я', '😀', '😃',
        '\U0001fae9', '\U0001cc00', '\ufffd', '\ufeff', '\u200d',
        'host:udp:private::high::1', 'srflx:tcp:public:private:low:passive:2']

def units(text):
    raw = text.encode('utf-16-le', errors='surrogatepass')
    return [int.from_bytes(raw[i:i+2], 'little') for i in range(0, len(raw), 2)]

valid = [units(text) for text in TEXT]
invalid = [[0xd800], [0xd801], [0xdc00], [0xdfff], [0xd800, 97], [0xfffd]]
cases = [{'locale': locale, 'strings': valid} for locale in LOCALES]
cases.append({'locale': 'en-US', 'strings': invalid})

def invoke(command, value):
    result = subprocess.run(command, input=json.dumps(value), capture_output=True,
                            text=True, check=True, timeout=20)
    return json.loads(result.stdout)

def check():
    node = os.environ.get('NODE', 'node')
    expected = invoke([node, str(ROOT / 'scripts/native_collation_reference.mjs')], cases)
    print(f'Oracle: Node {expected["versions"]["node"]}, ICU {expected["versions"]["icu"]}', flush=True)
    helper = runpy.run_path(str(ROOT / 'scripts/v3_native_fixture.py'))
    with tempfile.TemporaryDirectory(prefix='native-collation-') as directory:
        binary = helper['compile_collation_fixture'](
            ROOT / 'scripts/native_collation_fixture.rs', Path(directory))
        actual = invoke([str(binary)], cases)
        assert len(actual) == len(expected['rows']) == len(cases)
        for case, native, reference in zip(cases, actual, expected['rows']):
            assert len(native['signs']) == len(reference['signs']) == len(case['strings']) ** 2
            if case['strings'] is invalid:
                assert sum(value is None for value in native['signs']) == 35
                assert native['signs'][-1] == 0
            else:
                mismatches = [(i, a, b) for i, (a, b) in
                              enumerate(zip(native['signs'], reference['signs'])) if a != b]
                assert not mismatches, (case['locale'], mismatches[:8])
        records = check_records(binary, node, expected['defaultLocale'])
    print(f'PASS: {len(LOCALES) * len(valid) ** 2} ordering comparisons across {len(LOCALES)} locales')
    print(f'PASS: 35 malformed comparisons refused; {records} complete record comparisons')
    print(f'Oracle: Node {expected["versions"]["node"]}, ICU {expected["versions"]["icu"]}')

def check_records(binary, node, locale):
    shared = runpy.run_path(str(ROOT / 'scripts/check_v3_network_record.py'))
    selected = shared['cases']()
    inputs = [{'address': '192.0.2.1', 'related': '',
               'candidates': [[value[0] for value in row] for row in candidates],
               'histories': histories} for _, candidates, histories in selected]
    expected = invoke([node, str(ROOT / 'scripts/castle_v3_record_reference.mjs')], inputs)
    native_inputs = []
    for item in inputs:
        record = {**item, 'address': units(item['address']), 'related': units(item['related']),
                  'histories': [[units(text) for text in history] for history in item['histories']]}
        native_inputs.append({'locale': locale, 'record': record})
    actual = invoke([str(binary)], native_inputs)
    assert len(actual) == len(expected['rows']) == len(selected)
    compared = 0
    for (name, _, _), native, reference in zip(selected, actual, expected['rows']):
        if name == 'protocol_keys':
            assert native == {'error': 'IllFormedUtf16'}, native
        else:
            assert native == {'bytes': reference['bytes']}, name
            compared += 1
    return compared

if __name__ == '__main__':
    check()
