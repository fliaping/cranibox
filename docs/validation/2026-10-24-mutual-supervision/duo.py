"""Prototype: two processes (base, upper) that supervise and update each other.
Only process semantics are exercised (detach, restart, pointer switch, rollback).
Usage: duo.py <dir> run <role> <version> | duo.py <dir> harness <phase> [args]"""
import os, sys, time, json, subprocess, shutil

W = os.name == 'nt'
D = sys.argv[1]
ME = {'role': None, 'ver': None}
S = {'grace': 0.0, 'next': 0.0, 'fails': 0}
P = lambda *a: os.path.join(D, *a)
other = lambda r: 'upper' if r == 'base' else 'base'


def jread(p, default=None):
    try:
        with open(p) as f:
            return json.load(f)
    except Exception:
        return default


def jwrite(p, obj):
    t = p + '.tmp%d' % os.getpid()
    with open(t, 'w') as f:
        json.dump(obj, f)
    for _ in range(20):
        try:
            os.replace(t, p)
            return
        except PermissionError:
            time.sleep(0.02)


def event(*a):
    with open(P('events.log'), 'a') as f:
        f.write('%.2f %s\n' % (time.time() % 1000, ' '.join(str(x) for x in a)))


def hb(role):
    return jread(P('hb-%s.json' % role))


def up(role):
    h = hb(role)
    return bool(h) and time.time() - h['t'] < 1.5


def ptr(role):
    return jread(P('rt', role, 'current.json'))


def kill(pid):
    if W:
        subprocess.run(['taskkill', '/F', '/PID', str(pid)], capture_output=True)
    else:
        import signal
        try:
            os.kill(pid, signal.SIGKILL)
        except ProcessLookupError:
            pass


def spawn(role):
    p = ptr(role)
    cmd = [sys.executable, os.path.abspath(__file__), D, 'run', role, p['current']]
    log = open(P('log-%s.txt' % role), 'ab')
    kw = dict(stdin=subprocess.DEVNULL, stdout=log, stderr=log, close_fds=True)
    if W:
        flags = 0x00000008 | 0x00000200  # DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP
        try:
            subprocess.Popen(cmd, creationflags=flags | 0x01000000, **kw)  # CREATE_BREAKAWAY_FROM_JOB
            return 'breakaway'
        except OSError as e:
            subprocess.Popen(cmd, creationflags=flags, **kw)
            return 'NO-breakaway(%s)' % e.winerror
    subprocess.Popen(cmd, start_new_session=True, **kw)
    return 'setsid'


def activate(role, ver, beh):
    os.makedirs(P('rt', role, 'versions', ver), exist_ok=True)
    jwrite(P('rt', role, 'versions', ver, 'behavior.json'), beh)
    p = ptr(role)
    jwrite(P('rt', role, 'current.json'), {'current': ver, 'previous': p['current'], 'state': 'pending',
                                           'attempts': 0, 'failed': p.get('failed', [])})


def rollback(role, why):
    p = ptr(role)
    p['failed'].append(p['current'])
    p['current'], p['previous'], p['state'], p['attempts'] = p['previous'], None, 'confirmed', 0
    jwrite(P('rt', role, 'current.json'), p)
    event('rollback', role, 'to', p['current'], why)


def confirm(role, ver):
    p = ptr(role)
    if p and p['current'] == ver and p['state'] == 'pending':
        p['state'], p['attempts'] = 'confirmed', 0
        jwrite(P('rt', role, 'current.json'), p)
        event('confirmed', role, ver)


def beat():
    jwrite(P('hb-%s.json' % ME['role']), {'pid': os.getpid(), 'ver': ME['ver'], 't': time.time()})


def supervise(role):
    peer = other(role)
    now = time.time()
    if now < S['grace']:
        return
    if up(peer):
        S['fails'] = 0
        return
    if now < S['next']:
        return
    p = ptr(peer)
    if p['state'] == 'pending':
        p['attempts'] += 1
        jwrite(P('rt', peer, 'current.json'), p)
        if p['attempts'] > 3 and p['previous']:
            rollback(peer, 'crash-loop')
            p = ptr(peer)
    S['fails'] += 1
    mode = spawn(peer)
    S['next'] = time.time() + min(0.5 * 2 ** S['fails'], 4)
    S['grace'] = time.time() + 2.0
    event('respawn', peer, p['current'], mode, 'by', role)


def do_update(role, req):
    tgt = other(role)
    if not up(tgt) or ptr(tgt)['state'] != 'confirmed':
        return {'ok': False, 'reason': 'target not online and confirmed'}
    old = hb(tgt)
    activate(tgt, req['version'], req['behavior'])
    S['grace'] = time.time() + 120  # this actor owns the target during the update
    kill(old['pid'])
    time.sleep(0.3)
    mode = spawn(tgt)
    deadline = time.time() + req.get('window', 8)
    while time.time() < deadline:
        beat()
        p, h = ptr(tgt), hb(tgt)
        if up(tgt) and p['state'] == 'confirmed' and p['current'] == req['version'] and h['pid'] != old['pid']:
            S['grace'] = 0
            return {'ok': True, 'spawn': mode, 'now': req['version']}
        time.sleep(0.3)
    h = hb(tgt)
    if h and h['pid'] != old['pid']:
        kill(h['pid'])
    rollback(tgt, 'not-confirmed')
    spawn(tgt)
    end = time.time() + 8
    while time.time() < end:
        beat()
        h = hb(tgt)
        if up(tgt) and h['pid'] != old['pid'] and h['ver'] == old['ver']:
            break
        time.sleep(0.3)
    S['grace'] = 0
    h = hb(tgt)
    return {'ok': False, 'rolled_back': True, 'now': h and h['ver'], 'alive': up(tgt)}


def requests(role):
    d = P('req')
    for f in sorted(os.listdir(d)) if os.path.isdir(d) else []:
        if f.endswith('.json') and not os.path.exists(P('req', f[:-5] + '.res')):
            req = jread(P('req', f))
            if req and req['actor'] == role:
                jwrite(P('req', f[:-5] + '.res'), do_update(role, req))


def run(role, ver):
    ME.update(role=role, ver=ver)
    beh = jread(P('rt', role, 'versions', ver, 'behavior.json'), {})
    if beh.get('crash_immediately'):
        sys.exit(3)
    t0 = time.time()
    done = False
    while True:
        beat()
        if beh.get('crash_after') and time.time() - t0 > beh['crash_after']:
            sys.exit(4)
        if not done and not beh.get('never_confirm') and time.time() - t0 >= beh.get('confirm_after', 0.8):
            confirm(role, ver)
            done = True
        requests(role)
        supervise(role)
        time.sleep(0.3)


def wait(cond, secs):
    t = time.time()
    while time.time() - t < secs:
        if cond():
            return round(time.time() - t, 1)
        time.sleep(0.2)
    return None


def info():
    return {r: (hb(r) and {'pid': hb(r)['pid'], 'ver': hb(r)['ver'], 'age': round(time.time() - hb(r)['t'], 1)},
                ptr(r) and {k: ptr(r)[k] for k in ('current', 'previous', 'state', 'attempts', 'failed')})
            for r in ('base', 'upper')}


def harness(phase, a):
    assert 'duo-exp' in D
    if phase == 'init':
        for n in os.listdir(D):
            if n in ('rt', 'req') or n.startswith(('hb-', 'log-')) or n == 'events.log':
                shutil.rmtree(P(n), ignore_errors=True) if os.path.isdir(P(n)) else os.remove(P(n))
        for r in ('base', 'upper'):
            os.makedirs(P('rt', r, 'versions', '1'))
            jwrite(P('rt', r, 'versions', '1', 'behavior.json'), {})
            jwrite(P('rt', r, 'current.json'), {'current': '1', 'previous': None, 'state': 'confirmed', 'attempts': 0, 'failed': []})
        os.makedirs(P('req'))
        print('init ok; python', sys.version.split()[0], sys.platform)
    elif phase == 'start':  # the harness plays the role of the OS autostart entry: it only starts the base
        if a and a[0] == 'wmi':
            cmd = '"%s" "%s" "%s" run base %s' % (sys.executable, os.path.abspath(__file__), D, ptr('base')['current'])
            subprocess.run(['powershell', '-NoProfile', '-NonInteractive', '-Command',
                            "(Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{CommandLine='%s'}).ReturnValue" % cmd.replace("'", "''")],
                           capture_output=True)
            mode = 'wmi'
        else:
            mode = spawn('base')
        t = wait(lambda: up('base') and up('upper'), 12)
        print('start: spawn=%s both-up-after=%ss' % (mode, t), info())
    elif phase == 'status':
        print(info())
    elif phase == 'kill':
        r = a[0]
        old, other_old = hb(r)['pid'], hb(other(r))['pid']
        kill(old)
        t = wait(lambda: up(r) and hb(r)['pid'] != old, 14)
        print('killed %s: restarted-by-peer after %ss; peer survived=%s' % (r, t, hb(other(r))['pid'] == other_old and up(other(r))))
    elif phase == 'killboth':
        for r in ('base', 'upper'):
            kill(hb(r)['pid'])
        time.sleep(7)
        print('killboth: anything back after 7s? base=%s upper=%s (expect False False)' % (up('base'), up('upper')))
    elif phase == 'req':
        actor, ver, beh = a[0], a[1], json.loads(a[2])
        n = '%d' % (time.time() * 1000)
        jwrite(P('req', n + '.json'), {'actor': actor, 'version': ver, 'behavior': beh, 'window': 8})
        t = wait(lambda: os.path.exists(P('req', n + '.res')), 40)
        print('req %s updates %s -> %s:' % (actor, other(actor), ver), jread(P('req', n + '.res')), 'after', t, 's', info())
    elif phase == 'crashloop':
        r = a[0]
        old = hb(r)['pid']
        activate(r, 'loop', {'crash_after': 1.2, 'confirm_after': 2.0})
        kill(old)
        wait(lambda: False, 28)
        n = sum(1 for l in open(P('events.log')) if 'respawn %s loop' % r in l)
        print('crashloop %s: spawns of the bad version=%d' % (r, n), info())
    elif phase == 'jobinfo':
        import ctypes
        from ctypes import wintypes
        k = ctypes.windll.kernel32
        k.OpenProcess.restype = ctypes.c_void_p
        def injob(pid):
            h = k.OpenProcess(0x1000, 0, pid)
            r = wintypes.BOOL()
            ok = k.IsProcessInJob(ctypes.c_void_p(h), None, ctypes.byref(r)) if h else 0
            return bool(r.value) if ok else None
        buf = ctypes.create_string_buffer(144)
        ok = k.QueryInformationJobObject(None, 9, buf, 144, None)
        flags = int.from_bytes(buf.raw[16:20], 'little') if ok else None
        print('this process in a job:', injob(os.getpid()), 'limit flags:', hex(flags) if flags is not None else None,
              '(0x2000=KILL_ON_CLOSE 0x800=BREAKAWAY_OK 0x1000=SILENT_BREAKAWAY)')
        for r in ('base', 'upper'):
            h = hb(r)
            print(' ', r, 'pid', h and h['pid'], 'in a job:', h and injob(h['pid']))
    elif phase == 'events':
        print(''.join(open(P('events.log')).readlines()[-int(a[0]):]))
    elif phase == 'cleanup':
        for r in ('base', 'upper'):
            h = hb(r)
            if h:
                kill(h['pid'])
        time.sleep(1)
        shutil.rmtree(D, ignore_errors=True)
        print('cleanup: exists=%s' % os.path.exists(D))


if sys.argv[2] == 'run':
    run(sys.argv[3], sys.argv[4])
else:
    harness(sys.argv[3], sys.argv[4:])
