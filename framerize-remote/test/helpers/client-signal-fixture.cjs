let count = 0;
process.on('SIGINT', () => {
  count += 1;
  console.log(JSON.stringify({ kind: 'interrupt', count }));
});
process.stdin.setEncoding('utf8');
process.stdin.on('data', () => {
  console.log(JSON.stringify({ kind: 'result', count }));
  process.exit(count === 1 ? 0 : 44);
});
console.log(JSON.stringify({ kind: 'ready', pid: process.pid, parent: process.ppid,
  tty: Boolean(process.stdin.isTTY && process.stdout.isTTY) }));
