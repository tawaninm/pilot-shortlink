# Page: login ด้วย Google/GitHub และ Turnstile (เพิ่มใน `auth.md` และหน้าลืมรหัสผ่าน)

ที่มา: plan `pilot-oauth` ใน TAWAN-OS, `docs/requirements.md` AC-O*, AC-T*, AC-W1 (2026-11-19) ข้อความในเอกสารนี้ e2e ใช้หา element ต้องตรงทุกตัวอักษร

## หน้าเข้าสู่ระบบและสมัคร (`/login`, `/register`)

ใต้ปุ่ม primary และก่อนลิงก์สลับหน้า เพิ่มส่วน "หรือ":
1. เส้นคั่นพร้อมคำว่า "หรือ" (ข้อความธรรมดา ไม่ใช่ heading) ห่างปุ่ม primary `space-6`
2. ลิงก์ 2 อันเรียงลงมา ห่างกัน `space-3` หน้าตาแบบปุ่ม secondary เต็มความกว้าง:
   - "เข้าสู่ระบบด้วย Google" → `href="/api/auth/oauth/google/start"`
   - "เข้าสู่ระบบด้วย GitHub" → `href="/api/auth/oauth/github/start"`
   เป็น `<a>` ธรรมดา (top-level navigation) ไม่ใช่ fetch เพราะต้องพา browser ไปหน้า provider ข้อความเหมือนกันทั้งสองหน้า (บัญชีใหม่ถูกสร้างให้อัตโนมัติ)
3. ไม่ใช้โลโก้ของ Google/GitHub ในรอบนี้ (ไม่ต้องดูแนวทางแบรนด์ ไม่เพิ่มไฟล์รูป)

`/login?error=oauth`: แสดงบนหัวฟอร์มใน `role="alert"` ข้อความ "เข้าสู่ระบบด้วย Google หรือ GitHub ไม่สำเร็จ ลองอีกครั้งหรือใช้อีเมลและรหัสผ่าน" แล้วลบ `?error=oauth` ออกจากแถบที่อยู่ด้วย `history.replaceState` (รีเฟรชแล้วไม่ขึ้นซ้ำ)

## Turnstile (หน้าสมัคร `/register` และลืมรหัสผ่าน `/forgot-password`)

- หน้าเว็บเรียก `GET /api/config` ครั้งเดียวต่อการโหลดแอป
- `turnstileSiteKey` เป็น string: โหลด `https://challenges.cloudflare.com/turnstile/v0/api.js?render=explicit` เฉพาะสองหน้านี้ (ไม่โหลดทั้งแอป), render widget ใน `<div data-testid="turnstile">` เหนือปุ่ม primary ห่าง `space-4`, เก็บ token จาก callback, ปุ่ม primary กดไม่ได้จนกว่าจะมี token, หลังส่งฟอร์ม (สำเร็จหรือไม่) เรียก `turnstile.reset()` เพราะ token ใช้ได้ครั้งเดียว
- `turnstileSiteKey` เป็น `null` (โหมด fake ในเทสต์): ไม่โหลดสคริปต์ ไม่มี widget ส่ง `captcha: "fake-pass"`
- API ตอบ 403 `captcha_failed`: ข้อความใน `role="alert"` "ยืนยันว่าไม่ใช่บอทไม่สำเร็จ ลองอีกครั้ง" แล้ว reset widget
- widget มีชื่อที่ screen reader อ่านได้จาก Cloudflare เอง ไม่ต้องใส่ label เพิ่ม

## CSP

เพิ่มเฉพาะ `https://challenges.cloudflare.com` ใน `script-src` และ `frame-src` (และ `connect-src` ถ้า widget ต้องใช้) ไม่เปิด `unsafe-inline` หรือ `unsafe-eval`

## Route ฝั่ง backend

- `GET /api/auth/oauth/{google,github}/start` และ `/callback` (ไม่ต้องเสิร์ฟ SPA)
- `GET /api/config`
