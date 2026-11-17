# ผล spike ADR-002 (2026-11-17)

วัดบนบัญชี Workers Free ของตะวัน CPU อ่านจาก `wrangler tail` (`cpuTime`) ไม่ commit ไฟล์ tail ดิบเพราะมี header ของ request (มี IP)

## PBKDF2-HMAC-SHA256 ผ่าน WebCrypto (`hash-cpu/`, JS)
| iterations | CPU (ms) 3 ครั้ง | ผล |
|---|---|---|
| 10,000 | 2, 2, 2 | ok |
| 50,000 | 9, 10, 9 | ok |
| 100,000 | 19, 19, 19 | ok (เกิน 10 ms แต่ไม่โดน 1102) |
| 200,000 และ 600,000 | 0-1 | **500: `NotSupportedError: Pbkdf2 failed: iteration counts above 100000 are not supported`** |

WebCrypto บน Workers production จำกัด PBKDF2 ที่ 100,000 iterations ทำตามเกณฑ์ OWASP (600,000) ด้วย WebCrypto ไม่ได้ (`wrangler dev` ในเครื่องไม่จำกัด จึงไม่เห็นตอนทดสอบ local)

## ใน Wasm ของ Rust Worker (`hash-cpu-rs/`, `argon2` 0.6.0, `pbkdf2` 0.13.0)
| ค่า | CPU median (min-max) ms | n | ผล |
|---|---|---|---|
| noop | 0 (0-12) | 5 | ok |
| Argon2id m=4 MiB t=1 | 8 (4-18) | 5 | ok |
| Argon2id m=8 MiB t=1 | 11 (9-20) | 5 | ok |
| **Argon2id m=19 MiB t=2 (OWASP ขั้นต่ำ)** | **54 (39-63)** | 5 | ok |
| Argon2id m=46 MiB t=1 (OWASP ทางเลือก) | 76.5 (52-115) | 6 | ok |
| PBKDF2 (Rust) 10,000 | 19 (9-20) | 3 | ok |
| PBKDF2 (Rust) 100,000 | 106 (83-165) | 3 | ok |
| PBKDF2 (Rust) 600,000 | 502 (368-1337) | 3 | ok |

## สรุป
- Argon2id ตามเกณฑ์ OWASP ใช้ CPU ราว 54 ms ต่อครั้ง เกินเพดาน 10 ms ที่เอกสารระบุราว 5 เท่า แต่ทุก request ผ่าน ไม่มี Error 1102 ในการทดสอบนี้
- เอกสาร Cloudflare บอกว่ามีความยืดหยุ่น แต่ Worker ที่ "เกินเพดานสม่ำเสมอ" จะถูกตัด จึงยังไม่ใช่การรับประกัน
- ใน Wasm, Argon2id ถูกกว่า PBKDF2 ที่ระดับความปลอดภัยเท่ากันมาก (54 ms เทียบ 502 ms)
- รหัสผ่านถูก hash แค่ตอนสมัครและ login ไม่ใช่ทุก request ของแอป
