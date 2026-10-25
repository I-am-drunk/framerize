def make_plan(previous, request):
    stage = json.loads((BASE / 'package-stage-shared-v3-r1/inputs.json').read_text())
    release = Path(stage['release'])
    config = P / 'v3-worker-runtime-binding-r1/worker-config.json'
    worker_inputs = P / 'v3-authority-runtime-binding-r1/runtime-inputs.json'
    argv = list(previous['argv'])
    argv[0] = str(release / 'bin/framerize')
    argv.remove('--browser-request-ownership')
    argv[argv.index('--sidecar-script') + 1] = str(release / 'host/camoufox_host.py')
    argv += ['--offline-worker-config', str(config), '--offline-sdk', 'v3']
    plan = {key: previous[key] for key in ('purpose', 'execution_lock', 'capture_cleanup',
                                         'interruption')}
    plan.update(release=str(release), argv=argv, binary_sha256=sha(argv[0]),
        sidecar=str(release / 'host/camoufox_host.py'),
        sidecar_sha256=sha(release / 'host/camoufox_host.py'), request_binding=request,
        activation_nonce=uuid.uuid4().hex, activation_permit=str(HERE / 'output/activation/permit.json'),
        worker_config=dict(path=str(config), sha256=sha(config)),
        worker_inputs=dict(path=str(worker_inputs), sha256=sha(worker_inputs)))
    plan['package_files'] = [dict(path=r['path'], sha256=r['sha256'], size=r['size'],
                                 mode=r['release_mode']) for r in stage['files']]
    dropins = Path('/home/tjninja/.config/systemd/user/framerize.service.d')
    plan.update(proposed('admission.conf.proposed', configuration(argv, 'always')))
    plan['dropin'] = str(dropins / 'zzzzzzzzzzzzzz-shared-v3.conf')
    rollback = {key: previous[key] for key in ('argv', 'binary_sha256', 'sidecar', 'sidecar_sha256')}
    rollback.update(proposed('rollback.conf.proposed', configuration(rollback['argv'], 'no')))
    rollback['dropin'] = str(dropins / 'zzzzzzzzzzzzzzz-shared-v3-rollback.conf')
    plan['rollback'] = rollback
    plan['fence'] = proposed('activation-fence.conf.proposed', '[Unit]\nConditionPathExists=' +
        plan['activation_permit'] + '\n[Service]\nRestart=no\n')
    plan['fence']['dropin'] = str(dropins / 'zzzzzzzzzzzzzzzz-shared-v3-fence.conf')
    return plan
