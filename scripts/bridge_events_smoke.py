#!/usr/bin/env python3
"""Real server JSON-lines/event regressions; stdlib, unique socket and temp state."""
import json
import os
from pathlib import Path
import queue
import subprocess
import sys
import tempfile
import threading
import time

EXE = str(Path(sys.argv[1] if len(sys.argv) > 1 else
               'target/release/crew' + ('.exe' if os.name == 'nt' else '')).resolve())


def main():
    with tempfile.TemporaryDirectory(prefix='crew-bridge-') as tmp:
        root = Path(tmp)
        (root / 'config.toml').write_text('persist = false\n', encoding='utf-8')
        env = {k: v for k, v in os.environ.items() if not k.upper().startswith(('CREW', 'TMUXLIKE'))}
        env.update(CREW_SOCKET=f'crew-bridge-{os.getpid()}.sock', CREW_CONFIG=str(root / 'config.toml'),
                   CREW_STATE=str(root / 'state.json'), CREW_LOG=str(root / 'server.log'))

        def cli(*args):
            run = subprocess.run([EXE, *args], env=env, capture_output=True, text=True,
                                 encoding='utf-8', timeout=30)
            assert run.returncode == 0, (args, run.stdout, run.stderr)
            return json.loads(run.stdout)['result']

        bridge = None
        try:
            workspace = cli('workspace', 'create', 'bridge')
            pane = workspace['workspace']['tabs'][0]['panes'][0]['pane_id']
            bridge = subprocess.Popen([EXE, 'api', 'serve'], env=env, stdin=subprocess.PIPE,
                                      stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                      text=True, encoding='utf-8', bufsize=1)
            replies = queue.Queue()

            def read():
                for line in bridge.stdout:
                    replies.put(json.loads(line))

            reader = threading.Thread(target=read, daemon=True)
            reader.start()

            def send(ident, method, args=None):
                bridge.stdin.write(json.dumps({'id': ident, 'method': method,
                                               'params': {'args': args or []}}) + '\n')
                bridge.stdin.flush()

            send('snapshot', 'session.snapshot')
            assert replies.get(timeout=10)['id'] == 'snapshot'
            send('bad', 'plugin.unimplemented')
            assert replies.get(timeout=10)['error']['code'] == 'method_not_found'
            bridge.stdin.write('{bad json\n')
            bridge.stdin.flush()
            assert replies.get(timeout=10)['error']['code'] == 'invalid_request'
            print('ok strict request validation, ids and snapshots', flush=True)

            cli('pane', 'report-agent', pane, '--source', 'bridge', '--agent', 'fake', '--state', 'working')
            send('waiting', 'agent.wait', [pane, '--until', 'idle', '--timeout', '10000'])
            send('pong', 'ping')
            assert replies.get(timeout=5)['id'] == 'pong', 'wait blocked later requests'
            cli('pane', 'report-agent', pane, '--source', 'bridge', '--agent', 'fake', '--state', 'idle')
            reply = replies.get(timeout=15)
            assert reply['id'] == 'waiting' and 'result' in reply, reply
            print('ok independent requests progress during agent wait', flush=True)

            event = cli('events', 'list', '--limit', '512')
            old_epoch = event['epoch']
            send('events', 'events.wait', ['--after', str(event['latest_cursor']),
                                          '--epoch', old_epoch, '--timeout', '10000'])
            cli('workspace', 'rename', 'bridge', '橋接測試')
            reply = replies.get(timeout=15)
            assert reply['id'] == 'events' and reply['result']['events'], reply
            send('text', 'pane.read', [pane])
            reply = replies.get(timeout=10)
            assert reply['id'] == 'text' and isinstance(reply['result'], str), reply
            cli('server', 'stop')
            for _ in range(100):
                stopped = subprocess.run([EXE, 'session', 'list'], env=env,
                                         capture_output=True, timeout=5)
                if stopped.returncode != 0:
                    time.sleep(0.05)
                    break
                time.sleep(0.025)
            else:
                raise AssertionError('server did not stop')
            cli('workspace', 'create', 'restarted')
            send('epoch', 'events.list', ['--epoch', old_epoch])
            assert replies.get(timeout=10)['error']['code'] == 'server_restarted'
            print('ok events, text replies and restart epoch', flush=True)

            bridge.stdin.write('x' * (1024 * 1024 + 1) + '\n')
            bridge.stdin.flush()
            assert replies.get(timeout=15)['error']['code'] == 'request_too_large'
            send('after-large', 'ping')
            assert replies.get(timeout=10)['id'] == 'after-large'
            bridge.stdin.close()
            assert bridge.wait(timeout=10) == 0
            assert bridge.stderr.read() == ''
            print('PASS bridge/events smoke', flush=True)
        finally:
            if bridge and bridge.poll() is None:
                bridge.kill()
                bridge.wait(timeout=5)
            subprocess.run([EXE, 'kill-server'], env=env, capture_output=True, timeout=15)


if __name__ == '__main__':
    main()
