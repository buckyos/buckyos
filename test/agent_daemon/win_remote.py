#!/usr/bin/env python3
"""Drive a Windows test VM through the PVE guest agent.

Commands run through ``qm guest exec`` execute as SYSTEM in session 0, which
is neither the interactive user nor a desktop session. Daemon launches and
daemon CLI calls therefore go through Task Scheduler tasks registered for the
logged-in user with the interactive token only (``/RU user /IT``, no stored
password: a password-backed task fails to start with "element not found").
Output comes back through files under ``C:\\bk\\e2e``.

    win_remote.py <vmid> setup <user> <password>
    win_remote.py <vmid> write <remote path>            (content on stdin)
    win_remote.py <vmid> read <remote path>
    win_remote.py <vmid> system <powershell command>
    win_remote.py <vmid> user <powershell command>
    win_remote.py <vmid> launch <slot> <binary> <url path> [extra args...]
    win_remote.py <vmid> exited <slot>
"""
import json
import subprocess
import sys
import time

BASE = r'C:\bk\e2e'

RUNNER = r'''$ErrorActionPreference = 'Continue'
$base = 'C:\bk\e2e'
Remove-Item "$base\run.done" -ErrorAction SilentlyContinue
try { & "$base\run.ps1" *> "$base\run.out" } catch { $_ | Out-File "$base\run.out" -Append }
'ok' | Out-File "$base\run.done"
'''

DAEMON = r'''param([string]$Slot)
$ErrorActionPreference = 'Continue'
$base = 'C:\bk\e2e'
$spec = Get-Content "$base\launch-$Slot.json" -Raw | ConvertFrom-Json
$args = @()
if ($spec.extra) { $args += $spec.extra }
$args += @('run', '--url-stdin')
Set-Location $env:USERPROFILE
Get-Content $spec.url_file | & $spec.binary @args *> "$base\daemon-$Slot.log"
"$LASTEXITCODE" | Out-File "$base\daemon-$Slot.exit"
'''


def guest(vmid, args, stdin=None, timeout=120):
    cmd = ['qm', 'guest', 'exec', str(vmid), '--timeout', str(timeout)]
    if stdin is not None:
        cmd += ['--pass-stdin', '1']
    cmd += ['--'] + args
    out = subprocess.run(cmd, input=stdin, capture_output=True, text=True, timeout=timeout + 30)
    if out.returncode != 0 and not out.stdout.strip().startswith('{'):
        raise RuntimeError(out.stderr.strip() or out.stdout.strip())
    data = json.loads(out.stdout)
    return data.get('out-data', ''), data.get('err-data', ''), data.get('exitcode')


def system(vmid, script, timeout=120):
    return guest(vmid, ['powershell', '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-Command',
                        '[Console]::OutputEncoding=[Text.Encoding]::UTF8; ' + script], timeout=timeout)


def write(vmid, path, content):
    script = ('[Console]::InputEncoding=[Text.Encoding]::UTF8; $text = [Console]::In.ReadToEnd(); '
              f"[IO.File]::WriteAllText('{path}', $text, (New-Object Text.UTF8Encoding $false))")
    guest(vmid, ['powershell', '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-Command', script], stdin=content)


def read(vmid, path):
    out, err, code = system(vmid, f"if (Test-Path '{path}') {{ Get-Content -Raw '{path}' }} else {{ Write-Output '__MISSING__' }}")
    return None if out.strip() == '__MISSING__' else out


def schtask(vmid, name, command, user, password=None):
    out, err, code = guest(vmid, ['schtasks', '/Create', '/F', '/TN', name, '/TR', command, '/SC', 'ONCE', '/ST', '00:00',
                                  '/RU', user, '/IT', '/RL', 'LIMITED'])
    if code:
        raise RuntimeError(f'schtasks create failed: {out} {err}')
    out, err, code = guest(vmid, ['schtasks', '/Run', '/TN', name])
    if code:
        raise RuntimeError(f'schtasks run failed: {out} {err}')


def user(vmid, script, user_name, password, timeout=120):
    write(vmid, BASE + r'\run.ps1', script + '\n')
    system(vmid, f"Remove-Item '{BASE}\\run.done','{BASE}\\run.out' -ErrorAction SilentlyContinue")
    schtask(vmid, 'bk-e2e-run', f'powershell -NoProfile -ExecutionPolicy Bypass -File {BASE}\\runner.ps1', user_name, password)
    end = time.time() + timeout
    while time.time() < end:
        if read(vmid, BASE + r'\run.done') is not None:
            return read(vmid, BASE + r'\run.out') or ''
        time.sleep(1)
    raise TimeoutError('user command did not finish')


def main():
    vmid, command = sys.argv[1], sys.argv[2]
    rest = sys.argv[3:]
    if command == 'setup':
        user_name, password = rest
        system(vmid, f"New-Item -ItemType Directory -Force -Path '{BASE}' | Out-Null; "
                     f"icacls '{BASE}' /grant '{user_name}:(OI)(CI)F' | Out-Null")
        write(vmid, BASE + r'\runner.ps1', RUNNER)
        write(vmid, BASE + r'\daemon.ps1', DAEMON)
        write(vmid, BASE + r'\creds.json', json.dumps({'user': user_name, 'password': password}))
        print('ok')
        return
    creds = json.loads(read(vmid, BASE + r'\creds.json') or '{}')
    if command == 'write':
        write(vmid, rest[0], sys.stdin.read())
    elif command == 'read':
        text = read(vmid, rest[0])
        if text is None:
            sys.exit(3)
        sys.stdout.write(text)
    elif command == 'system':
        out, err, code = system(vmid, rest[0])
        sys.stdout.write(out + err)
        sys.exit(code or 0)
    elif command == 'user':
        sys.stdout.write(user(vmid, rest[0], creds['user'], creds['password']))
    elif command == 'launch':
        slot, binary, url_file, *extra = rest
        write(vmid, f'{BASE}\\launch-{slot}.json', json.dumps({'binary': binary, 'url_file': url_file, 'extra': extra}))
        system(vmid, f"Remove-Item '{BASE}\\daemon-{slot}.exit','{BASE}\\daemon-{slot}.log' -ErrorAction SilentlyContinue")
        schtask(vmid, f'bk-e2e-daemon-{slot}', f'powershell -NoProfile -ExecutionPolicy Bypass -File {BASE}\\daemon.ps1 {slot}',
                creds['user'], creds['password'])
    elif command == 'exited':
        text = read(vmid, f'{BASE}\\daemon-{rest[0]}.exit')
        print('' if text is None else text.strip())
    else:
        raise SystemExit(__doc__)


if __name__ == '__main__':
    main()
