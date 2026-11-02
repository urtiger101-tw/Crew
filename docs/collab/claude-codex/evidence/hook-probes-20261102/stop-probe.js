// Probe-only Claude Code Stop hook (isolated test harness; not a receiver).
// argv: <logDir> <mode: log|block> <markerPath>
// stdin: hook input JSON (captured verbatim to logDir).
// block mode: emit {"decision":"block","reason":...} exactly once, only when the
// marker file exists AND stop_hook_active is not true; then delete the marker.
const fs = require('fs');
const path = require('path');

const [, , logDir, mode, markerPath] = process.argv;
let raw = '';
process.stdin.setEncoding('utf8');
process.stdin.on('data', (chunk) => { raw += chunk; });
process.stdin.on('end', () => {
  fs.mkdirSync(logDir, { recursive: true });
  const stamp = `${Date.now()}-${process.pid}`;
  fs.writeFileSync(path.join(logDir, `stop-${stamp}.json`), raw);
  const logLine = (msg) => fs.appendFileSync(
    path.join(logDir, 'hook-invocations.log'),
    `${new Date().toISOString()} pid=${process.pid} mode=${mode} ${msg}\n`);

  let input = {};
  try { input = JSON.parse(raw); } catch (err) { logLine(`parse-error=${err.message}`); }
  if (mode !== 'block') { logLine('action=log-only'); return; }

  const pending = fs.existsSync(markerPath);
  const active = input.stop_hook_active === true;
  if (pending && !active) {
    fs.unlinkSync(markerPath);
    logLine(`action=block pending=true stop_hook_active=${active} marker=cleared`);
    process.stdout.write(JSON.stringify({
      decision: 'block',
      reason: 'INBOX: reply with the word PINEAPPLE',
    }));
    return;
  }
  logLine(`action=allow pending=${pending} stop_hook_active=${active}`);
});
