# PRD-lite: pilot-shortlink

วันที่: 2026-11-17 | ระดับ: 2 | ผู้ทำ: Claude Code (ขั้น 0 ของ `web-build-workflow`) | สถานะ: ตะวันยืนยันแล้ว 2026-11-17, เพิ่มยืนยัน email ลืมรหัสผ่าน และ QR 2026-11-19

## 1. Problem
งานนี้มีสองเป้าหมาย: (1) ทดสอบ `web-build-workflow` ตั้งแต่ขั้น 0 ถึง deploy จริงด้วย stack สอง profile (2) ได้แอปย่อลิงก์ที่ตะวันใช้ได้จริง เช่นลิงก์ portfolio หรือ CV ที่อยากรู้ว่ามีคนเปิดจากที่ไหน

แอปเดียวกันทำสองครั้ง ใช้ frontend, ดีไซน์ และ `api/openapi.yaml` ชุดเดียวกัน ต่างกันที่ backend:
- **A:** Cloudflare Workers + Rust (`workers-rs`) + D1
- **B:** Go container บน Render + Neon Postgres

## 2. Goals และตัวชี้วัด
- ทั้ง A และ B deploy แล้วเปิดใช้งานได้จริง และผ่าน e2e ชุดเดียวกัน
- redirect ตอบภายใน 200 ms ที่ p95 เมื่อ instance ตื่นอยู่ (B บน Render free sleep ได้ วัดหลังตื่น)
- Lighthouse หน้าหลัก performance และ accessibility อย่างน้อย 90
- ทุกเคสใน `docs/test-plan.md` ผ่าน
- บันทึกจุดที่ skill ขาดลง `scratch/tasks/web-build-workflow/plan.md` (ปิดแล้ว ย้ายไป `99_ARCHIVE/scratch_history/2026-11-18_web-build-workflow/` ของ TAWAN-OS)

## 3. User stories
P0
1. สมัครด้วย email + รหัสผ่าน, login, logout
2. สร้างลิงก์สั้นจาก URL ปลายทาง ไม่ใส่ alias ได้รหัสสุ่ม 7 ตัว ใส่ alias ได้ตามที่ขอถ้ายังว่าง
3. ดูรายการลิงก์ของตัวเอง (ใหม่สุดก่อน) พร้อมจำนวนคลิกรวม
4. ลบลิงก์ของตัวเอง
5. ใครก็ได้เปิด `/<code>` แล้วถูกพาไปปลายทาง และระบบนับคลิก
6. ดูสถิติลิงก์ของตัวเอง: คลิกรวม, กราฟรายวัน 30 วันล่าสุด, referrer host 10 อันดับ

P1
7. แก้ปลายทางของลิงก์
8. ปุ่มคัดลอกลิงก์สั้น

P1 รอบเพิ่มระบบเสริม (ตะวันสั่ง 2026-11-19, plan `pilot-addons` ใน TAWAN-OS)
9. ยืนยัน email หลังสมัคร ก่อนยืนยันจะ login และดูลิงก์เดิมได้ แต่สร้าง แก้ หรือลบลิงก์ไม่ได้
10. ลืมรหัสผ่าน: ขอลิงก์รีเซ็ตทางอีเมลแล้วตั้งรหัสใหม่
11. ดาวน์โหลด QR code ของลิงก์สั้นเป็นไฟล์ SVG

P1 รอบ OAuth และกันบอท (ตะวันสั่ง 2026-11-19, plan `pilot-oauth` ใน TAWAN-OS)
12. เข้าสู่ระบบหรือสมัครด้วย Google หรือ GitHub โดยไม่ต้องตั้งรหัสผ่าน ถ้าอีเมลตรงกับบัญชีเดิมที่ provider ยืนยันแล้ว ได้บัญชีเดิม
13. ฟอร์มสมัครและลืมรหัสผ่านมี Cloudflare Turnstile กันบอท

## 4. Requirements
- **URL ปลายทาง:** รับเฉพาะ `http` และ `https`, ยาวไม่เกิน 2048 ตัวอักษร, ปฏิเสธ URL ที่ชี้กลับมาที่โดเมนของแอปเอง (กัน redirect วน)
- **Alias:** 3-32 ตัว ใช้ได้ `a-z 0-9 -` ไม่ขึ้นต้นหรือลงท้ายด้วย `-` ไม่สนตัวพิมพ์ใหญ่เล็ก (`CV` กับ `cv` คือตัวเดียวกัน) ห้ามคำสงวน: `api`, `app`, `admin`, `login`, `logout`, `register`, `static`, `assets`, `health`, `openapi`, `verify-email`, `forgot-password`, `reset-password` (สามคำหลังเป็นหน้าเว็บที่เพิ่ม 2026-11-19)
- **Alias ซ้ำ:** สองคนขอชื่อเดียวกันพร้อมกัน ต้องมีคนเดียวได้ อีกคนได้ 409 (unique constraint ในฐานข้อมูล ไม่ใช่เช็กก่อนแล้วค่อยเขียน)
- **Redirect:** ตอบ 302 พร้อม `Cache-Control: no-store` เพื่อให้นับคลิกได้ทุกครั้ง ลิงก์ที่ไม่มีหรือถูกลบแล้วตอบหน้า 404
- **การนับคลิก:** ไม่เก็บ IP, User-Agent เต็ม หรือข้อมูลที่ระบุตัวคน เก็บแค่เวลา (ปัดเป็นวัน UTC) และ referrer host ลบลิงก์แล้วข้อมูลคลิกของลิงก์นั้นหายด้วย
- **Auth:** รหัสผ่านเก็บแบบ hash (Argon2id), ยาวอย่างน้อย 10 ตัว, session เป็น cookie `HttpOnly; Secure; SameSite=Lax` หมดอายุ 7 วัน, logout ทำให้ session ใช้ไม่ได้ทันที, request ที่เปลี่ยนข้อมูลต้องตรวจ header `Origin`
- **สิทธิ์:** ผู้ใช้เห็น แก้ ลบ และดูสถิติได้เฉพาะลิงก์ของตัวเอง ลิงก์ของคนอื่นตอบ 404 (ไม่บอกว่ามีอยู่)
- **Rate limit:** สร้างลิงก์ไม่เกิน 30 ครั้งต่อนาทีต่อบัญชี, login ผิดไม่เกิน 5 ครั้งต่อ 15 นาทีต่อ email
- **Token ในอีเมล (ยืนยัน email และรีเซ็ตรหัสผ่าน):** สุ่ม 32 byte จาก CSPRNG, ฐานข้อมูลเก็บแค่ SHA-256, หมดอายุ 30 นาที, ใช้ได้ครั้งเดียว, ขอ token ใหม่แล้ว token เก่าของจุดประสงค์เดียวกันใช้ไม่ได้, token ผิด หมดอายุ หรือใช้แล้วได้ 400 `token_invalid` ข้อความเดียวกัน, ลิงก์ในอีเมลสร้างจาก `APP_ORIGIN` ใน config ไม่ใช่ header ของ request, หน้าเว็บส่ง token ด้วย POST (ไม่ยืนยันด้วย GET เพราะตัวสแกนลิงก์ในกล่องจดหมายจะเปิดลิงก์เอง), log ห้ามมี token
- **ยืนยัน email:** สมัครแล้วส่งอีเมลยืนยันทันที, ผู้ใช้ที่ยังไม่ยืนยันเรียก endpoint ที่สร้าง แก้ หรือลบลิงก์ได้ 403 `email_unverified`, ขอส่งซ้ำได้ไม่เกิน 3 ครั้งต่อชั่วโมงต่อบัญชี, ยืนยันแล้วขอส่งซ้ำได้ 204 โดยไม่ส่งอีเมล, บัญชีที่มีอยู่ก่อนงานนี้ถือว่ายืนยันแล้ว
- **ลืมรหัสผ่าน:** หน้าขอลิงก์ตอบ 204 ข้อความเดียวกันไม่ว่ามี email นั้นหรือไม่, ส่งอีเมลรีเซ็ตได้ไม่เกิน 3 ฉบับต่อชั่วโมงต่อ email ครั้งที่เกินตอบ 204 เหมือนเดิมแต่ไม่ส่งอีเมลและไม่ยกเลิกลิงก์เดิม (ถ้าตอบ 429 คนอื่นจะใช้อีเมลเราขอซ้ำจนเราขอเองไม่ได้ ตัดสิน 2026-11-19), ตั้งรหัสใหม่แล้วลบ session ทั้งหมดของบัญชีนั้น, รีเซ็ตสำเร็จถือว่ายืนยัน email แล้ว (เพราะพิสูจน์ว่าเข้ากล่องจดหมายได้)
- **ส่งอีเมล:** ผ่าน Resend ส่งเบื้องหลังหลังบันทึกข้อมูลแล้ว, Resend ล่มแล้วการสมัครหรือการขอลิงก์ยังสำเร็จ (ผู้ใช้กดส่งใหม่ได้), ใช้ header `Idempotency-Key`
- **QR:** สร้างในเบราว์เซอร์จาก URL ลิงก์สั้น ไม่ต้องมี endpoint ใหม่ ดาวน์โหลดเป็น SVG ชื่อ `<code>.svg`
- **OAuth (Google, GitHub):** Authorization Code + PKCE S256, `state` ใช้ครั้งเดียวอายุ 10 นาทีผูกกับ cookie `__Host-oauth` ของ browser ที่เริ่ม, `redirect_uri` สร้างจาก `APP_ORIGIN`, ตัวระบุบัญชีคือ (provider, subject) ไม่ใช่อีเมล, ผูกกับบัญชีอีเมลเดียวกันอัตโนมัติเฉพาะเมื่อ provider ยืนยันอีเมล (Google `email_verified`, GitHub อีเมล `primary` และ `verified` จาก `/user/emails`) ไม่อย่างนั้นปฏิเสธ (ตัดสิน D1 2026-11-19), ถ้าบัญชีเดิมยังไม่ยืนยันอีเมล ตอนผูกให้ล้างรหัสผ่าน (เป็น `!oauth`) ลบ session และ token อีเมลที่ยังไม่ใช้ทั้งหมดของบัญชีนั้น เพราะคนสมัครอาจไม่ใช่เจ้าของอีเมล (pre-hijack, พบจาก security review 2026-11-19), บัญชีที่สร้างจาก OAuth ถือว่ายืนยัน email แล้วและไม่มีรหัสผ่าน (ตั้งได้ผ่านลืมรหัสผ่าน), จบที่ `/` หรือ `/login?error=oauth` เท่านั้น ไม่รับ URL ปลายทางจาก query, log ห้ามมี code, token, state หรืออีเมล
- **Turnstile:** ฟอร์มสมัครและลืมรหัสผ่านส่ง `captcha` ตรวจที่ server ก่อนแตะข้อมูลบัญชีและก่อนนับโควตาต่ออีเมลของลืมรหัสผ่าน (captcha ที่ไม่ผ่านไม่กินโควตาของเจ้าของอีเมล) ไม่มีหรือไม่ผ่านได้ 403 `captcha_failed` ไม่ใส่ในหน้า login (มี rate limit อยู่แล้ว ตัดสิน D6 2026-11-19) sitekey สาธารณะส่งให้หน้าเว็บผ่าน `GET /api/config`
- **ภาษา UI:** ไทย

## 5. UX notes
- ออกแบบในขั้น 2 ตาม `uxui-workflow` ห้ามยืมดีไซน์จากโปรเจกต์อื่น (Fresh Slate)
- หน้า: สมัคร/เข้าสู่ระบบ, รายการลิงก์ + ฟอร์มสร้าง, สถิติลิงก์, 404
- state ที่ต้องมี: ยังไม่มีลิงก์, กำลังโหลด, error จาก API, alias ซ้ำ, ถึง rate limit
- WCAG 2.2 AA: ใช้คีย์บอร์ดได้ครบ, contrast อย่างน้อย 4.5:1, ข้อความ error ผูกกับช่องกรอก

## 6. Constraints
- frontend, ดีไซน์ และ OpenAPI ใช้ร่วมกันระหว่าง A และ B ต่างกันแค่ backend และ host
- free tier เท่านั้น: Workers Free + D1, Render Free + Neon Free, Resend Free (3,000 ฉบับต่อเดือน, 100 ฉบับต่อวัน)
- ไม่ซื้อโดเมน (ตะวันเลือก 2026-11-19): ผู้ส่งเป็น `onboarding@resend.dev` ซึ่ง Resend ส่งได้แค่ถึงอีเมลของเจ้าของบัญชี Resend ผู้ใช้คนอื่นบน production สมัครได้แต่จะไม่ได้รับอีเมล จนกว่าจะมีโดเมนที่ยืนยันกับ Resend แล้ว
- เทสต์อัตโนมัติไม่เรียก Resend จริง ใช้ adapter `capture` ที่เปิดด้วย `MAIL_MODE=capture` เท่านั้น
- เทสต์อัตโนมัติไม่เรียก Google, GitHub หรือ Cloudflare จริง ใช้ fake IdP (`OAUTH_MODE=fake`, `OAUTH_FAKE_BASE`) และ `TURNSTILE_MODE=fake` ที่เปิดได้เฉพาะ `APP_ORIGIN` เป็น localhost
- Secret ใหม่ต่อ backend ตะวันตั้งเอง: `GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET`, `GITHUB_CLIENT_ID`, `GITHUB_CLIENT_SECRET`, `TURNSTILE_SECRET` และค่าสาธารณะ `TURNSTILE_SITE_KEY`; GitHub OAuth App และ Google OAuth client อย่างละ 1 ตัวใช้ร่วมกันทั้ง A และ B โดยใส่ redirect URI สองอัน (GitHub ให้ได้สูงสุด 10 อัน ปิด wildcard และ Device Flow) Google สถานะ Testing ขอแค่ `openid email profile` ผู้ทดสอบต้องอยู่ใน Test users
- โปรเจกต์อยู่ที่ `D:\TAWAN-OS\pilot-shortlink\` นอก Obsidian vault
- ไม่เดาแทนตะวันเรื่องบัญชีและ login ของ Cloudflare, Render, Neon, GitHub

## 7. Out of scope
ทีมหรือการแชร์ลิงก์, custom domain, วันหมดอายุของลิงก์, สถิติประเทศหรืออุปกรณ์, การชำระเงิน, เปลี่ยน email, ลบบัญชี, outbox และ retry การส่งอีเมลอัตโนมัติ, ยกเลิกการผูกบัญชี OAuth, provider อื่นนอกจาก Google และ GitHub, Turnstile ในหน้า login

## 8. Assumptions และ open questions
- ใช้โดเมน `*.workers.dev` (A) และ `*.onrender.com` (B) ไม่ซื้อโดเมน
- Open: board Trello อยู่ใน workspace "Animation" ไปก่อน

## 9. Acceptance criteria
1. Given ผู้ใช้ใหม่ When สมัครด้วย email ที่ยังไม่มีและรหัส 10 ตัวขึ้นไป Then ได้ session และเข้าหน้ารายการลิงก์
2. Given email ที่มีแล้ว When สมัครซ้ำ Then ได้ 409 และข้อความว่า email นี้ถูกใช้แล้ว
3. Given login แล้ว When สร้างลิงก์โดยไม่ใส่ alias Then ได้รหัส 7 ตัว `[a-z0-9]` และเปิดรหัสนั้นแล้วได้ 302 ไปปลายทาง
4. Given alias `tawan-cv` ว่างอยู่ When สองคนขอพร้อมกัน Then 201 หนึ่งคน 409 หนึ่งคน
5. Given alias `API` When สร้าง Then ได้ 422 เพราะเป็นคำสงวน (ไม่สนตัวพิมพ์)
6. Given URL `javascript:alert(1)` หรือ URL ของแอปเอง When สร้าง Then ได้ 422
7. Given ลิงก์ของ user A When user B ขอดู แก้ ลบ หรือดูสถิติ Then ได้ 404 ทุกกรณี
8. Given ลิงก์ที่ถูกเปิด 3 ครั้งจาก `https://www.facebook.com/x` When เจ้าของดูสถิติ Then คลิกรวม 3, วันนี้ 3, referrer `www.facebook.com` 3
9. Given ตาราง clicks ในฐานข้อมูล When ตรวจคอลัมน์ Then ไม่มี IP หรือ User-Agent
10. Given logout แล้ว When ใช้ cookie เดิมเรียก API Then ได้ 401
11. Given ส่งคำขอสร้างลิงก์ 31 ครั้งในนาทีเดียว When ครั้งที่ 31 Then ได้ 429
12. Given A และ B deploy แล้ว When รัน e2e ชุดเดียวกันกับทั้งสอง URL Then ผ่านทั้งคู่

ยืนยัน email
- AC-V1 Given สมัครใหม่ When เรียก `GET /api/me` Then `emailVerified: false` และมีอีเมลยืนยัน 1 ฉบับถึง email นั้น
- AC-V2 Given token จากอีเมล When `POST /api/auth/verify-email` Then 204 และ `emailVerified: true`
- AC-V3 Given token ที่ใช้แล้วหรือหมดอายุ When ยืนยัน Then 400 `token_invalid`; token ผิดรูปแบบ (ไม่ใช่ 43 ตัว base64url) ได้ 422 `invalid_body` ตามการตรวจ body ทั่วไป
- AC-V4 Given ยังไม่ยืนยัน When สร้าง แก้ หรือลบลิงก์ Then 403 `email_unverified`
- AC-V5 Given ขอส่งซ้ำ 3 ครั้งในชั่วโมงเดียว When ขอครั้งที่ 4 Then 429
- AC-V6 Given ขอส่งซ้ำแล้ว When ใช้ token ฉบับก่อน Then 400 `token_invalid`
- AC-V7 Given token เดียวกัน When ยืนยันพร้อมกัน 2 request Then 204 หนึ่งครั้ง 400 หนึ่งครั้ง
- AC-V8 Given ไม่ได้ตั้ง `MAIL_MODE=capture` When เรียก `/api/test/mail` Then 404
- AC-V9 Given ตาราง token ในฐานข้อมูล When ตรวจหลังเทสต์ Then ไม่มี token แบบ plain text
- AC-V10 Given เทสต์ AC 1-12 เดิม When รันกับระบบที่มีการยืนยัน email Then ผ่านทั้งหมด
- AC-V11 Given สมัครผ่านหน้าเว็บ When เปิดลิงก์ในอีเมล Then เห็นหน้ายืนยันสำเร็จ และแถบเตือนหายไป
- AC-V12 Given production When ตะวันสมัครด้วยอีเมลของบัญชี Resend Then ได้อีเมลจริงและกดยืนยันผ่าน

ลืมรหัสผ่าน
- AC-R1 Given email ที่มีและไม่มีในระบบ When `POST /api/auth/password/forgot` Then 204 ทั้งคู่ และมีอีเมลส่งออกเฉพาะ email ที่มีบัญชี
- AC-R2 Given token รีเซ็ต When `POST /api/auth/password/reset` ด้วยรหัสใหม่ 10 ตัวขึ้นไป Then 204, login ด้วยรหัสใหม่ได้ รหัสเดิมไม่ได้
- AC-R3 Given login ค้างไว้ 2 session When รีเซ็ตสำเร็จ Then cookie เดิมทั้งสองได้ 401
- AC-R4 Given token รีเซ็ตที่ใช้แล้วหรือหมดอายุ, หรือ token ยืนยัน email When รีเซ็ต Then 400 `token_invalid`
- AC-R5 Given ขอลิงก์ 3 ครั้งในชั่วโมงเดียว When ขอครั้งที่ 4 ด้วย email เดิม Then 204, ไม่มีอีเมลฉบับที่ 4 และลิงก์ฉบับที่ 3 ยังใช้ได้
- AC-R6 Given บัญชีที่ยังไม่ยืนยัน email When รีเซ็ตสำเร็จ Then `emailVerified: true`
- AC-R7 Given รหัสใหม่สั้นกว่า 10 ตัว When รีเซ็ต Then 422 และ token ยังใช้ได้

QR
- AC-Q1 Given ลิงก์ในรายการ When กดดาวน์โหลด QR Then ได้ไฟล์ `<code>.svg` และถอดรหัส QR แล้วได้ URL ลิงก์สั้นตรงทุกตัวอักษร
- AC-Q2 Given ใช้คีย์บอร์ดอย่างเดียว When Tab ไปที่ปุ่ม QR แล้วกด Enter Then ดาวน์โหลดได้ และปุ่มมีชื่อที่ screen reader อ่านได้
- AC-O1 Given อีเมลใหม่ When login ด้วย Google สำเร็จ Then ได้บัญชีใหม่ `emailVerified: true` มี session และ 302 ไป `/`
- AC-O2 Given GitHub คืนอีเมลหลายอัน When login ด้วย GitHub Then ใช้เฉพาะอีเมลที่ `primary` และ `verified`
- AC-O3 Given บัญชีรหัสผ่านเดิมอีเมลเดียวกัน และ provider ยืนยันอีเมลแล้ว When login ด้วย OAuth Then ได้บัญชีเดิม (ลิงก์เดิมยังอยู่)
- AC-O4 Given provider บอกว่าอีเมลยังไม่ยืนยัน หรือ GitHub ไม่มีอีเมล verified When callback Then ไม่ผูก ไม่สร้างบัญชี 302 `/login?error=oauth`
- AC-O5 Given state ไม่ตรง cookie, ไม่มี cookie, state ใช้แล้ว หรือหมดอายุ When callback Then 302 `/login?error=oauth` ไม่มี session
- AC-O6 Given เริ่ม login When ดู authorize URL และ token request Then มี `code_challenge_method=S256`, `redirect_uri` = `<APP_ORIGIN>/api/auth/oauth/<provider>/callback` และ `code_verifier` ตรงกับ challenge
- AC-O7 Given callback ที่มี `return_to` หรือ URL อื่นใน query When สำเร็จ Then ไป `/` เท่านั้น
- AC-O8 Given บัญชีที่สร้างจาก OAuth When login ด้วยรหัสผ่านใดๆ Then 401; When ลืมรหัสผ่านแล้วตั้งรหัส Then login ด้วยรหัสใหม่ได้
- AC-O9 Given login ด้วย (provider, subject) เดิมแต่อีเมลที่ provider ส่งเปลี่ยนไป When callback Then ได้บัญชีเดิม
- AC-O10 Given ผู้ใช้กดยกเลิกที่หน้า provider (`error=access_denied`) หรือ provider ล่ม When callback Then 302 `/login?error=oauth` ไม่มี 500
- AC-O11 Given production (ไม่มี `OAUTH_MODE`) When เริ่ม login ด้วย Google Then Location ขึ้นต้นด้วย `https://accounts.google.com/` และ provider อื่นนอกจาก google, github ได้ 404
- AC-O12 Given คนอื่นสมัครด้วยอีเมลเราไว้ก่อนแต่ยังไม่ยืนยัน (มีรหัสผ่านและ session ของเขา) When เรา login ด้วย provider ที่ยืนยันอีเมลนี้ Then ได้บัญชีนั้น แต่รหัสผ่านเดิม login ไม่ได้ (401) และ session เดิมใช้ไม่ได้; บัญชีที่ยืนยันอีเมลแล้วผูกได้โดยรหัสผ่านเดิมยังใช้ได้
- AC-T4 Given ส่งลืมรหัสผ่านด้วย captcha ผิดหลายครั้ง When ส่งด้วย captcha ถูก Then ยังได้อีเมลรีเซ็ต (captcha ที่ผิดไม่นับในโควตา 3 ฉบับต่อชั่วโมง)
- AC-T1 Given ไม่มี captcha หรือ captcha ไม่ผ่าน When สมัครหรือขอลิงก์รีเซ็ต Then 403 `captcha_failed` ไม่สร้างบัญชีและไม่ส่งอีเมล
- AC-T2 Given captcha ผ่าน When สมัครหรือขอลิงก์รีเซ็ต Then ทำงานตามเดิม
- AC-T3 Given หน้าสมัครและลืมรหัสผ่าน When เปิดหน้า Then มี widget, CSP เพิ่มเฉพาะ `https://challenges.cloudflare.com`, axe ไม่มี violation
- AC-W1 Given หน้า login และสมัคร When กดปุ่ม "เข้าสู่ระบบด้วย Google" หรือ "เข้าสู่ระบบด้วย GitHub" Then ไปถึงรายการลิงก์, และ `/login?error=oauth` แสดงข้อความผิดพลาดใน `role="alert"`
