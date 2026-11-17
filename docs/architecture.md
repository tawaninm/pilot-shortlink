# Architecture: pilot-shortlink

วันที่: 2026-11-17 | ระดับ: 2 | Profile: A และ B | สถานะ: ตะวันอนุมัติ 2026-11-17 ยกเว้น ADR-002 ที่รอผลวัด CPU

## 1. ภาพรวม
แอปย่อลิงก์ที่มีบัญชีผู้ใช้และสถิติคลิก frontend และ API contract ใช้ร่วมกัน backend ทำสองแบบเพื่อเทียบ stack ทั้งสองแบบเสิร์ฟ frontend และ API จาก origin เดียวกัน จึงไม่ต้องตั้ง CORS และ cookie `SameSite=Lax` ใช้ได้ตรงๆ

```text
A:  Browser -> Cloudflare Worker (Rust) -+-> static assets (web/dist)
                                          +-> /api/*  -> D1 (SQLite)
                                          +-> /{code} -> 302 + นับคลิก (ctx.waitUntil)

B:  Browser -> Render web service (Go) --+-> static assets (embed web/dist)
                                          +-> /api/*  -> Neon Postgres (pgx pool)
                                          +-> /{code} -> 302 + นับคลิก
```

## 2. Stack
| ส่วน | ใช้ | เวอร์ชัน (ตรวจ 2026-11-17) | เหตุผล |
|---|---|---|---|
| Frontend | React + TypeScript + Vite, TanStack Router + Query, Tailwind 4, shadcn | 19.3.1, 7.0.2, 8.3.8, 1.170.46 / 5.107.1, 4.3.5, 4.23.0 | ตาม `stack.md` |
| API client | openapi-typescript + openapi-fetch | 7.13.0 / 0.17.0 | type จาก `api/openapi.yaml` ไม่เขียนซ้ำ |
| A backend | Rust `worker` (workers-rs) + D1 | 0.9.0 | Rust เป็นภาษาหลักของ Workers รันใน free plan |
| B backend | Go + chi + sqlc + pgx + goose | 1.27.4, v5.5.0, v1.31.1, v5.13.0, v3.30.1 | net/http-compatible, SQL เขียนเองแต่ได้ type |
| B OpenAPI | ใช้ `api/openapi.yaml` เป็นตัวจริง ไม่สร้างจากโค้ด | | สอง backend ต้องตรงกับไฟล์เดียว จึงไม่ใช้ Huma (Huma สร้าง spec จากโค้ด ทำให้มีสองแหล่ง) |
| Test | Vitest, Playwright, `cargo test`, `go test` + testcontainers-go | 5.1.0, 1.65.1, v0.45.0 | |
| Package manager | pnpm | 12.21.0 | ไม่ติดตั้งรุ่นที่ออกไม่ถึง 1 วัน |

## 3. โครง repo
```text
api/openapi.yaml        สัญญากลาง (ตัวจริง)
web/                    frontend เดียว
backend-a/              Rust Worker + migrations ของ D1 + wrangler.toml
backend-b/              Go service + migrations (goose) + sqlc queries + Dockerfile
contract/               test เรียก API จริงตาม openapi.yaml รันได้กับทั้ง A และ B (Vitest + openapi-fetch)
e2e/                    Playwright ชุดเดียว รันกับ URL ของ A หรือ B
design/                 tokens.json, build-tokens.mjs, tokens.css, preview.html
docs/                   requirements, research, architecture, test-plan, design/
```

## 4. API contract
ตัวจริงคือ `api/openapi.yaml` (ผ่าน Redocly lint 2.79.0) สรุป:

| Method | Path | ใครเรียกได้ | ผลลัพธ์ | Idempotent |
|---|---|---|---|---|
| POST | `/api/auth/register` | ทุกคน | 201 + cookie / 409 / 422 / 429 | ไม่ (email ซ้ำได้ 409) |
| POST | `/api/auth/login` | ทุกคน | 200 + cookie / 401 / 429 | ใช่ |
| POST | `/api/auth/logout` | มี session | 204 | ใช่ |
| GET | `/api/me` | มี session | 200 / 401 | ใช่ |
| GET | `/api/links` | เจ้าของ | 200 รายการพร้อม `last30Days` | ใช่ |
| POST | `/api/links` | เจ้าของ | 201 / 409 / 422 / 429 | ไม่ (alias ซ้ำได้ 409, ไม่มี alias สร้างลิงก์ใหม่ทุกครั้ง) |
| PATCH | `/api/links/{code}` | เจ้าของ | 200 / 404 | ใช่ |
| DELETE | `/api/links/{code}` | เจ้าของ | 204 / 404 | ครั้งที่สองได้ 404 (ผลต่อข้อมูลเท่าเดิม) |
| GET | `/api/links/{code}/stats` | เจ้าของ | 200 / 404 | ใช่ |
| GET | `/{code}` | ทุกคน | 302 / 404 | นับคลิกทุกครั้งโดยตั้งใจ |

- error ใช้ `application/problem+json` (RFC 9457) พร้อม `code` และ `field` ตามที่ frontend ต้องเลือกข้อความใน `docs/design/flows.md`
- 429 มี `Retry-After`
- ลิงก์ที่ไม่มีและลิงก์ของคนอื่นตอบ 404 เหมือนกันทุกไบต์
- สถิติคืนในการเรียกครั้งเดียว ทำให้ state "ข้อมูลมาไม่ครบ" ของหน้าสถิติไม่เกิด (ตอบคำถามที่ค้างจาก `flows.md`)

## 5. Data model (เหมือนกันทั้ง D1 และ Postgres)
| ตาราง | คอลัมน์หลัก | key และ index | หมายเหตุ |
|---|---|---|---|
| `users` | id, email (ตัวพิมพ์เล็ก), password_hash, created_at | unique(email) | |
| `sessions` | token_hash (SHA-256 ของ token), user_id, expires_at | pk(token_hash), index(user_id) | logout ลบแถว, cookie เก็บ token จริง ฐานข้อมูลเก็บแค่ hash |
| `links` | id, user_id, code (ตัวพิมพ์เล็ก), url, created_at | unique(code), index(user_id, created_at desc) | ลบ user ลบลิงก์ตาม |
| `click_daily` | link_id, day (UTC), referrer_host ('' คือไม่ทราบ), clicks | pk(link_id, day, referrer_host) | เก็บแค่ยอดรวมต่อวัน ไม่มีแถวต่อคลิก ไม่มี IP หรือ User-Agent ลบลิงก์ลบตาม |
| `rate_limits` | key (เช่น `create:<user_id>`, `login:<email>`), window_start, count | pk(key, window_start) | fixed window ลบแถวเก่าตอนเขียน |

- **alias ซ้ำพร้อมกัน:** insert ตรงๆ แล้วจับ unique violation ตอบ 409 ไม่เช็กก่อนเขียน
- **รหัสสุ่ม:** 7 ตัว `[a-z0-9]` จาก CSPRNG ชนแล้วสุ่มใหม่สูงสุด 5 ครั้ง (36^7 ราว 7.8 หมื่นล้านแบบ)
- **นับคลิก:** `INSERT ... ON CONFLICT (link_id, day, referrer_host) DO UPDATE SET clicks = clicks + 1` เป็น atomic ทั้ง SQLite และ Postgres
- **migration:** D1 ใช้ไฟล์ SQL ของ wrangler, B ใช้ goose ทุกไฟล์มี down (ระดับ 2)

## 6. Auth และสิทธิ์
- session cookie `session=<32 byte base64url>; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=604800`
- request ที่เปลี่ยนข้อมูล (POST, PATCH, DELETE) ตรวจ `Origin` ต้องตรงกับ origin ของแอป ไม่ตรงตอบ 403 (กัน CSRF ร่วมกับ SameSite)
- ทุก query ของลิงก์ใส่ `WHERE user_id = ?` ฝั่ง server ไม่พึ่ง frontend
- login ผิดตอบ 401 ข้อความเดียวกันไม่ว่า email มีหรือไม่ และใช้เวลาใกล้กัน (hash รหัสหลอกเมื่อไม่มี email)
- hash รหัสผ่าน: ดู ADR-002

## 7. Threat model (ระดับ 2 แบบย่อ)
| สิ่งที่ต้องปกป้อง | ภัย | ผลกระทบ | การป้องกัน | เทสต์ใน `test-plan.md` |
|---|---|---|---|---|
| ลิงก์และสถิติของผู้ใช้ | user B อ่าน/แก้/ลบของ user A (A01) | ข้อมูลรั่ว | `WHERE user_id`, 404 เหมือนกัน | สองบัญชี ทุก endpoint |
| รหัสผ่าน | ฐานข้อมูลรั่ว, เดารหัส (A04, A07) | บัญชีถูกยึด | hash ตาม ADR-002, rate limit login | login ผิด 6 ครั้ง ได้ 429 |
| session | ขโมย cookie, CSRF (A01, A07) | ทำแทนเจ้าของ | HttpOnly, Secure, SameSite=Lax, ตรวจ Origin | POST ที่ Origin ผิดได้ 403 |
| redirect | ใช้ย่อลิงก์ phishing หรือ `javascript:` (A05) | ผู้คลิกโดนหลอก | รับแค่ http/https, ห้ามชี้กลับโดเมนตัวเอง, rate limit การสร้าง | URL `javascript:` ได้ 422 |
| ระบบ | สร้างลิงก์รัวจน D1 เต็มโควตา (A06) | ใช้งานไม่ได้ทั้งวัน | rate limit 30 ต่อนาที | ครั้งที่ 31 ได้ 429 |
| dependency | แพ็กเกจ npm ถูกยึด (A03) | โค้ดอันตรายใน build | lockfile, pnpm minimumReleaseAge, Renovate รอ 3 วัน | CI ใช้ `--frozen-lockfile` |
| error | stack trace หลุด (A10) | เปิดเผยโครงสร้างภายใน | error เป็น problem+json, log รายละเอียดฝั่ง server | 500 ไม่มี stack trace |

## 8. Observability (ระดับ 2)
- log เป็น JSON หนึ่งบรรทัดต่อ request: method, path pattern (ไม่ใส่ code หรือ email), status, ms ห้าม log cookie, รหัสผ่าน, token
- A: Workers Logs ของ Cloudflare / B: stdout ที่ Render เก็บ
- error tracking (Sentry หรือเทียบเท่า): [TBC] ต้องใช้บัญชีของตะวัน ถ้าไม่ตั้งให้บันทึกเป็นข้อจำกัดของ pilot
- uptime check: `/health` ทั้งสองฝั่ง

## 9. CI (GitHub Actions)
ทุก PR: `pnpm install --frozen-lockfile`, `node design/build-tokens.mjs --check`, `ds-check`, Biome, typecheck, Vitest, `cargo clippy` + `cargo test`, `go vet` + `go test`, contract test กับ A (wrangler dev + D1 local) และ B (Postgres ใน container) action ทั้งหมด pin ด้วย commit SHA และให้ `permissions:` น้อยที่สุด

## 10. ADR

### ADR-001: frontend และ API อยู่ origin เดียวกัน
- บริบท: auth ใช้ cookie และ requirements ต้องการ SameSite=Lax
- ตัวเลือก: (1) origin เดียว (2) frontend แยกบน Workers + API ข้ามโดเมนพร้อม CORS
- เลือก (1): ไม่ต้องตั้ง CORS และ cookie ข้ามโดเมน ลดจุดตั้งค่าผิด B ต้องเสิร์ฟไฟล์ static เอง (Go `embed`) ซึ่งทำได้ในไม่กี่บรรทัด
- ผล: build frontend ก่อน backend ทุกครั้ง

### ADR-002: hash รหัสผ่านบน A (รอตะวันตัดสิน)
- บริบท: OWASP ให้ใช้ Argon2id (19 MiB, 2 iterations) หรือ PBKDF2-HMAC-SHA256 600,000 iterations แต่ Workers Free ให้ CPU 10 ms ต่อ request ยังไม่มีตัวเลขวัดจริง (`docs/research.md` ข้อ 4)
- ตัวเลือก:
  1. Workers Paid ($5/เดือน): ได้ Argon2id ตาม OWASP ผิดข้อจำกัด free tier
  2. ลดค่า hash ให้ทัน 10 ms: ไม่เพิ่มต้นทุน แต่ต่ำกว่าเกณฑ์ OWASP ต้องเขียนเป็นข้อจำกัดของ pilot
  3. Passkey (WebAuthn) บน A: ไม่มีรหัสผ่านให้รั่ว ตรวจลายเซ็นใช้ CPU น้อย แต่ A กับ B จะมี flow login ต่างกัน และเปลี่ยน requirements ข้อ 3
  4. บริการ auth ภายนอก: hash ถูกต้องแต่เพิ่มบัญชีและ dependency และ A กับ B ต่างกัน
- ขั้นตอนก่อนตัดสิน: deploy Worker ทดสอบที่วัด CPU ของ PBKDF2 ผ่าน WebCrypto ที่ 100,000 และ 600,000 iterations และ Argon2id ใน Wasm (ต้องใช้บัญชี Cloudflare ของตะวัน หรือ `wrangler deploy --temporary` ซึ่งยังไม่รู้ว่าโควตา CPU เท่า Free หรือไม่)
- B ใช้ Argon2id (m=19456 KiB, t=2, p=1) ตาม OWASP ไม่ขึ้นกับ ADR นี้
- **ผลวัด 2026-11-17** (`spikes/RESULTS.md`): Argon2id ตาม OWASP ใน Rust Wasm ใช้ CPU ราว 54 ms (median) บน Free ผ่านทุก request ไม่มี 1102 แต่เกินเพดาน 10 ms ที่เอกสารระบุ, WebCrypto PBKDF2 ถูกจำกัดที่ 100,000 iterations จึงทำตาม OWASP ไม่ได้, PBKDF2 ใน Rust ต้องใช้ราว 500 ms ตัวเลือกที่ 2 (ลดค่า hash) จึงเหลือแค่ Argon2id m=4 MiB t=1 (8 ms) ซึ่งต่ำกว่าเกณฑ์ OWASP ชัดเจน

### ADR-003: ใช้ `api/openapi.yaml` เป็นแหล่งเดียว ไม่สร้าง spec จากโค้ด
- บริบท: มีสอง backend ต่างภาษา
- ตัวเลือก: (1) เขียน spec มือเป็นตัวจริงแล้วตรวจด้วย contract test (2) สร้าง spec จากโค้ด Go ด้วย Huma
- เลือก (1): ถ้าสร้างจากโค้ดฝั่งหนึ่ง อีกฝั่งต้องตามฝั่งนั้น และ spec เปลี่ยนตามโค้ดโดยไม่มีคนตั้งใจ contract test ชุดเดียวจับได้ทั้งสองฝั่ง
- ผล: ไม่ใช้ Huma ใน pilot นี้ (ต่างจาก `stack.md` ที่แนะนำ Huma สำหรับโปรเจกต์ backend เดียว)
