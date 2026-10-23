from pathlib import Path
from contextlib import closing
import re
import sqlite3
import subprocess
import uuid
import json
from .support import read_json, require, sha

ADDRESS = ('127.0.0.1', 8788)


def properties(unit):
    raw = subprocess.check_output(['systemctl', '--user', 'show', unit, '-p',
        'MainPID,InvocationID,ActiveState,ControlGroup,Restart,RuntimeMaxUSec'],
        text=True, timeout=5, stderr=subprocess.DEVNULL)
    return dict(line.split('=', 1) for line in raw.splitlines())


def arguments(pid):
    return (Path('/proc') / str(pid) / 'cmdline').read_bytes().decode().rstrip('\0').split('\0')


def argument(argv, name):
    require(argv.count(name) == 1 and argv.index(name) + 1 < len(argv), 'missing_launch_argument')
    return argv[argv.index(name) + 1]


def account_rows(path):
    with closing(sqlite3.connect(Path(path).as_uri() + '?mode=ro', uri=True, timeout=2)) as db:
        return db.execute("SELECT id,json_extract(json,'$.framer_user_id') FROM accounts").fetchall()


def separate_accounts(native, browser, account, user):
    require(native == [(account, user)], 'native_account_inventory')
    require(all(isinstance(row[1], str) and row[1] for row in browser),
            'browser_account_identity_missing')
    require(all(account != row[0] and user != row[1] for row in browser), 'browser_account_overlap')


def separate_roots(native, browser):
    require(native.is_absolute() and native.resolve(strict=True) == native, 'native_root')
    require(browser.is_absolute() and browser.resolve(strict=True) == browser, 'browser_root')
    require(not native.is_relative_to(browser) and not browser.is_relative_to(native), 'browser_path_overlap')


def support_bindings(plan, root):
    name = plan['support_manifest']
    require(re.fullmatch(r'runtime/cooldown-support-[0-9a-f]{16}/runtime-sources.json', name),
            'support_manifest_path')
    require(name in plan['files'], 'support_manifest_missing')
    digest = plan['files'][name]
    require(Path(name).parent.name == 'cooldown-support-' + digest[:16], 'support_directory')
    entries = read_json(root / name, digest=digest)
    require(isinstance(entries, dict) and entries, 'support_manifest_empty')
    for child, expected in entries.items():
        require(Path(child).name == child, 'support_member_path')
        member = str(Path(name).parent / child)
        require(plan['files'].get(member) == expected, 'support_member_missing')


def listener_owned(pid):
    rows = Path('/proc/net/tcp').read_text().splitlines()[1:]
    inodes = {row.split()[9] for row in rows
        if row.split()[1] == '0100007F:2254' and row.split()[3] == '0A'}
    require(len(inodes) == 1, 'isolated_listener_missing_or_ambiguous')
    links = {path.readlink().as_posix() for path in (Path('/proc') / str(pid) / 'fd').iterdir()}
    require('socket:[' + next(iter(inodes)) + ']' in links, 'isolated_listener_not_owned')


class Binding:
    def __init__(self, plan_path, digest):
        self.plan = read_json(plan_path, digest=digest)
        self.unit = self.plan['unit']
        require(re.fullmatch(r'framerize-browserless-[a-z0-9-]+\.service', self.unit), 'isolated_unit_name')
        require(re.fullmatch(r'[0-9a-f]{32}', self.plan['invocation_id']), 'invocation_id')
        self.root = Path(self.plan['root'])
        self.browser = properties('framerize.service')
        require(self.browser['InvocationID'] == self.plan['browser_invocation_id'], 'browser_generation')
        browser_args = arguments(self.browser['MainPID'])
        self.browser_db = Path(argument(browser_args, '--db'))
        separate_roots(self.root, self.browser_db.parent)
        separate_roots(self.root, Path(argument(browser_args, '--profiles')))
        source = self.plan['private_input']
        require(Path(source['path']).is_relative_to(self.root), 'private_input_scope')
        self.input = read_json(source['path'], private=True, digest=source['sha256'])
        require(set(self.input) == {'account_id', 'framer_user_id', 'context'}, 'private_input_shape')
        self.account = self.input['account_id']
        require(str(uuid.UUID(self.account)) == self.account, 'account_id')
        require(isinstance(self.input['framer_user_id'], str) and
                0 < len(self.input['framer_user_id']) <= 128, 'framer_user_id')
        require(isinstance(self.input['context'], dict), 'request_context')
        require(len(json.dumps(self.input['context']).encode()) <= 8192, 'context_byte_limit')
        self.current = None
        self.refresh()

    def refresh(self):
        current = properties(self.unit)
        require(current['ActiveState'] == 'active' and
                current['InvocationID'] == self.plan['invocation_id'], 'native_generation')
        require(current['Restart'] == 'no' and current['RuntimeMaxUSec'] != 'infinity',
                'native_lifetime_policy')
        require(properties('framerize.service') == self.browser, 'browser_generation_changed')
        if self.current is not None:
            require(current == self.current, 'native_generation_changed')
        self.group = Path('/sys/fs/cgroup' + current['ControlGroup'])
        require(self.group.name == self.unit and self.group.resolve() == self.group, 'native_cgroup')
        required = {'bin/framerize', 'runtime/camoufox_host.py',
                    'runtime/worker-config.json', 'runtime/native-routing.json'}
        require(required <= set(self.plan['files']), 'missing_file_pin')
        for name, digest in self.plan['files'].items():
            path = self.root / name
            require(path.is_relative_to(self.root) and path.resolve(strict=True) == path, 'file_pin_path')
            require(name == 'bin/framerize' or name.startswith('runtime/'), 'file_pin_scope')
            require(sha(path) == digest, 'runtime_file_changed')
        support_bindings(self.plan, self.root)
        candidates = []
        for pid in (self.group / 'cgroup.procs').read_text().split():
            executable = Path('/proc') / pid / 'exe'
            if executable.resolve() == self.root / 'bin/framerize':
                require(sha(executable) == self.plan['files']['bin/framerize'], 'native_executable')
                candidates.append(int(pid))
        require(len(candidates) == 1, 'native_process_identity')
        self.pid = candidates[0]
        argv = arguments(self.pid)
        expected = {'--listen': '127.0.0.1:8788', '--db': str(self.root / 'framerize.db'),
            '--profiles': str(self.root / 'profiles'), '--eligible-accounts': self.account,
            '--active-pool-size': '1',
            '--sidecar-script': str(self.root / 'runtime/camoufox_host.py'),
            '--offline-worker-config': str(self.root / 'runtime/worker-config.json'),
            '--offline-routing-config': str(self.root / 'runtime/native-routing.json')}
        for name, value in expected.items():
            require(argument(argv, name) == value, 'native_launch_binding')
        listener_owned(self.pid)
        separate_accounts(account_rows(self.root / 'framerize.db'),
            account_rows(self.browser_db), self.account, self.input['framer_user_id'])
        routing = read_json(self.root / 'runtime/native-routing.json')
        require(routing['max_chunks'] == 1 and routing['continuation'] is None
                and routing['allow_paid'] is False, 'native_attempt_policy')
        self.current = current
        self.sample()

    def sample(self):
        names = ('memory.current', 'memory.peak', 'memory.swap.current', 'pids.current', 'pids.peak')
        values = {name: int((self.group / name).read_text()) for name in names}
        maximum = int((self.group / 'memory.max').read_text())
        require(maximum == self.plan['memory_max_bytes'] and 0 < maximum <= 1024**3, 'memory_cap')
        require(int((self.group / 'memory.swap.max').read_text()) == 0, 'swap_cap')
        require(values['memory.swap.current'] == 0, 'swap_used')
        require(int((self.group / 'pids.max').read_text()) == 32, 'task_cap')
        events = dict(line.split() for line in (self.group / 'memory.events').read_text().splitlines())
        require(all(int(value) == 0 for value in events.values()), 'memory_event')
        return values
