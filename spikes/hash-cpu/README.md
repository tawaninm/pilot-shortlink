# ADR-002 spike: CPU ของ hash รหัสผ่านบน Workers Free

ผลในเครื่อง (`wrangler dev`, ไม่บังคับ CPU limit) 2026-11-17:
- `/pbkdf2?it=100000` และ `/pbkdf2?it=600000` ทำงานได้ (WebCrypto native) ยังไม่รู้ CPU จริงบน Free และไม่รู้ว่า production จำกัดจำนวน iteration หรือไม่
- `/argon2` ล้ม: `WebAssembly.compile(): Wasm code generation disallowed by embedder` Workers ไม่ให้ compile Wasm ตอนรัน `hash-wasm` จึงใช้ไม่ได้ Argon2id ต้องมาจาก Wasm ที่ build ไว้ก่อน (Rust `workers-rs` + crate `argon2`) วัดใน spike Rust แยก

วัดบน Free (หลังตะวัน `wrangler login`):
```bash
npm install --ignore-scripts
npx wrangler@4.163.0 deploy
npx wrangler@4.163.0 tail --format json      # อีกหน้าต่าง อ่าน cpuTime ของแต่ละ request
curl https://<worker>.workers.dev/noop
curl "https://<worker>.workers.dev/pbkdf2?it=100000"
curl "https://<worker>.workers.dev/pbkdf2?it=600000"
```
request ที่เกิน CPU limit ได้ Error 1102
