// Spike for ADR-002: how much CPU does password hashing cost on Workers Free (10 ms CPU per request)?
// Code cannot time itself on Workers (the clock only advances on I/O), so CPU is read from `wrangler tail`
// (event.cpuTime) and from whether the request fails with Error 1102 (CPU limit exceeded).
//   GET /noop                      baseline
//   GET /pbkdf2?it=100000          PBKDF2-HMAC-SHA256 via WebCrypto (native)
//   GET /argon2?m=19456&t=2        Argon2id via hash-wasm (WebAssembly), m in KiB
import { argon2id } from 'hash-wasm';

const PASSWORD = new TextEncoder().encode('correct horse battery staple');

async function pbkdf2(iterations) {
  const key = await crypto.subtle.importKey('raw', PASSWORD, 'PBKDF2', false, ['deriveBits']);
  const salt = crypto.getRandomValues(new Uint8Array(16));
  await crypto.subtle.deriveBits({ name: 'PBKDF2', hash: 'SHA-256', salt, iterations }, key, 256);
}

async function argon(memoryKiB, iterations) {
  await argon2id({
    password: PASSWORD,
    salt: crypto.getRandomValues(new Uint8Array(16)),
    parallelism: 1,
    iterations,
    memorySize: memoryKiB,
    hashLength: 32,
    outputType: 'hex',
  });
}

export default {
  async fetch(request) {
    const url = new URL(request.url);
    const n = (k, d) => Number(url.searchParams.get(k) ?? d);
    try {
      if (url.pathname === '/noop') return new Response('ok noop\n');
      if (url.pathname === '/pbkdf2') { await pbkdf2(n('it', 100000)); return new Response(`ok pbkdf2 it=${n('it', 100000)}\n`); }
      if (url.pathname === '/argon2') { await argon(n('m', 19456), n('t', 2)); return new Response(`ok argon2id m=${n('m', 19456)} t=${n('t', 2)}\n`); }
      return new Response('not found\n', { status: 404 });
    } catch (e) {
      return new Response(`error ${e.name}: ${e.message}\n`, { status: 500 });
    }
  },
};
