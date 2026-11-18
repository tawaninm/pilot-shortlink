# PRD-lite: pilot-shortlink

วันที่: 2026-11-17 | ระดับ: 2 | ผู้ทำ: Claude Code (ขั้น 0 ของ `web-build-workflow`) | สถานะ: ตะวันยืนยันแล้ว 2026-11-17

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

P2 (ไม่ทำใน pilot)
9. QR code, ลืมรหัสผ่าน, ยืนยัน email

## 4. Requirements
- **URL ปลายทาง:** รับเฉพาะ `http` และ `https`, ยาวไม่เกิน 2048 ตัวอักษร, ปฏิเสธ URL ที่ชี้กลับมาที่โดเมนของแอปเอง (กัน redirect วน)
- **Alias:** 3-32 ตัว ใช้ได้ `a-z 0-9 -` ไม่ขึ้นต้นหรือลงท้ายด้วย `-` ไม่สนตัวพิมพ์ใหญ่เล็ก (`CV` กับ `cv` คือตัวเดียวกัน) ห้ามคำสงวน: `api`, `app`, `admin`, `login`, `logout`, `register`, `static`, `assets`, `health`, `openapi`
- **Alias ซ้ำ:** สองคนขอชื่อเดียวกันพร้อมกัน ต้องมีคนเดียวได้ อีกคนได้ 409 (unique constraint ในฐานข้อมูล ไม่ใช่เช็กก่อนแล้วค่อยเขียน)
- **Redirect:** ตอบ 302 พร้อม `Cache-Control: no-store` เพื่อให้นับคลิกได้ทุกครั้ง ลิงก์ที่ไม่มีหรือถูกลบแล้วตอบหน้า 404
- **การนับคลิก:** ไม่เก็บ IP, User-Agent เต็ม หรือข้อมูลที่ระบุตัวคน เก็บแค่เวลา (ปัดเป็นวัน UTC) และ referrer host ลบลิงก์แล้วข้อมูลคลิกของลิงก์นั้นหายด้วย
- **Auth:** รหัสผ่านเก็บแบบ hash (Argon2id), ยาวอย่างน้อย 10 ตัว, session เป็น cookie `HttpOnly; Secure; SameSite=Lax` หมดอายุ 7 วัน, logout ทำให้ session ใช้ไม่ได้ทันที, request ที่เปลี่ยนข้อมูลต้องตรวจ header `Origin`
- **สิทธิ์:** ผู้ใช้เห็น แก้ ลบ และดูสถิติได้เฉพาะลิงก์ของตัวเอง ลิงก์ของคนอื่นตอบ 404 (ไม่บอกว่ามีอยู่)
- **Rate limit:** สร้างลิงก์ไม่เกิน 30 ครั้งต่อนาทีต่อบัญชี, login ผิดไม่เกิน 5 ครั้งต่อ 15 นาทีต่อ email
- **ภาษา UI:** ไทย

## 5. UX notes
- ออกแบบในขั้น 2 ตาม `uxui-workflow` ห้ามยืมดีไซน์จากโปรเจกต์อื่น (Fresh Slate)
- หน้า: สมัคร/เข้าสู่ระบบ, รายการลิงก์ + ฟอร์มสร้าง, สถิติลิงก์, 404
- state ที่ต้องมี: ยังไม่มีลิงก์, กำลังโหลด, error จาก API, alias ซ้ำ, ถึง rate limit
- WCAG 2.2 AA: ใช้คีย์บอร์ดได้ครบ, contrast อย่างน้อย 4.5:1, ข้อความ error ผูกกับช่องกรอก

## 6. Constraints
- frontend, ดีไซน์ และ OpenAPI ใช้ร่วมกันระหว่าง A และ B ต่างกันแค่ backend และ host
- free tier เท่านั้น: Workers Free + D1, Render Free + Neon Free
- โปรเจกต์อยู่ที่ `D:\TAWAN-OS\pilot-shortlink\` นอก Obsidian vault
- ไม่เดาแทนตะวันเรื่องบัญชีและ login ของ Cloudflare, Render, Neon, GitHub

## 7. Out of scope
ทีมหรือการแชร์ลิงก์, custom domain, QR, วันหมดอายุของลิงก์, สถิติประเทศหรืออุปกรณ์, ลืมรหัสผ่าน, ยืนยัน email, การชำระเงิน

## 8. Assumptions และ open questions
- ใช้โดเมน `*.workers.dev` (A) และ `*.onrender.com` (B) ไม่ซื้อโดเมน
- ข้อจำกัดที่รู้แล้ว: ไม่มีลืมรหัสผ่าน ผู้ใช้ที่ลืมรหัสต้องสมัครใหม่ ยอมรับได้สำหรับ pilot
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
