# Case study: pilot-shortlink

ร่าง 2026-11-18 ตัวเลขหลัง deploy เป็น `[TBC]` จนกว่าจะทำขั้น 8

- **เป้าหมายและปัญหา:**
  1. ทดสอบ workflow สร้างเว็บของ TAWAN-OS (`web-build-workflow`) ตั้งแต่ requirement ถึง deploy กับงานจริง
  2. ได้แอปย่อลิงก์ที่ใช้เองได้ เช่นลิงก์ portfolio หรือ CV ที่อยากรู้ว่ามีคนเปิดจากที่ไหน
- **ระยะเวลา:** 2026-11-17 ถึง [TBC] (ขั้น 0-7 ใช้ราว 2 วัน)
- **ประเภท:** ฝึกเอง
- **ทีม:** ตะวัน (ตัดสินใจทุก gate), Claude Code (ออกแบบ, รีวิว, เขียน backend B และหน้าเว็บ), agy (เขียน backend A)
- **บทบาทของตะวัน:**
  - ยืนยัน requirements
  - เลือกทิศทางดีไซน์
  - อนุมัติ architecture และตัดสิน ADR-002 หลังเห็นผลวัด CPU
  - ตอบ Proceed ก่อนเริ่มเขียนโค้ด
  - สั่งให้ทำ A กับ B คู่ขนาน
- **ผลลัพธ์ (ในเครื่องและ CI):**
  - backend สองตัวผ่าน contract test ชุดเดียวกัน 57/57
  - e2e 20/20 ทั้งคู่ (desktop และจอ 320px)
  - axe ไม่เจอปัญหา WCAG 2.2 AA
  - CI 7 job ผ่าน
  - A deploy แล้ว: https://pilot-shortlink-a.pilot-shortlink-hash-cpu-spike.workers.dev (2026-11-18) smoke test และ e2e 20/20 ผ่านบน production
  - B: [TBC] รอบัญชี Render และ Neon
  - redirect p95 และ Lighthouse: [TBC]

## 1. ปัญหาและความท้าทาย
แอปเดียวต้องทำสองครั้งด้วย backend ต่างกัน:
- **A:** Cloudflare Workers + Rust + D1
- **B:** Go + Postgres บน Render

ทั้งสองใช้หน้าเว็บ ดีไซน์ และ `api/openapi.yaml` ชุดเดียวกัน โจทย์คือทำให้ "เหมือนกัน" ตรวจได้ด้วยเครื่อง ไม่ใช่ด้วยการอ่านโค้ด

## 2. วิธีหาปัญหา
- **ขั้น 0:** grilling ทีละคำถามจนได้ acceptance criteria 12 ข้อ ทุกข้อตรวจได้
- **ขั้น 1:** วิจัยคู่แข่ง 3 ราย และข้อจำกัดของ Workers
- **ขั้น 2:** ทำ flow พร้อม state matrix 28 แถวก่อนออกแบบหน้าจอ เจอ state ที่เกือบลืม 3 กรณี:
  - session หมดอายุกลางทาง
  - กดส่งซ้ำ
  - ลบลิงก์จากอีกแท็บ

## 3. ทางเลือกและเหตุผล
| เรื่อง | ทางเลือก | เลือก | เพราะ |
|---|---|---|---|
| hash รหัสผ่านบน Workers Free (ADR-002) | PBKDF2 ผ่าน WebCrypto / Argon2id ค่าเบา / Argon2id ตาม OWASP / Workers Paid | Argon2id ตาม OWASP บน Free และเฝ้าดู Error 1102 | วัดบนบัญชีจริงแล้ว Argon2id ใช้ CPU ราว 54 ms ผ่านทุกครั้ง ส่วน WebCrypto ถูกจำกัดที่ 100,000 iterations จึงทำตาม OWASP ไม่ได้ และ `wrangler dev` ในเครื่องไม่แสดงข้อจำกัดนี้ |
| แหล่งความจริงของ API (ADR-003) | สร้าง spec จากโค้ด Go (Huma) / เขียน OpenAPI เอง | เขียนเอง | สอง backend ต้องตรงกับไฟล์เดียว |
| หน้าเว็บกับ API (ADR-001) | คนละ origin / origin เดียว | origin เดียว | cookie แบบ SameSite=Lax ใช้ได้ตรงๆ ไม่ต้องตั้ง CORS |
| ทำงานคู่ขนาน | agy ทำทั้งหมด / แบ่ง A-B | agy ทำ A, Claude ทำ B | ตะวันสั่งเพื่อให้เสร็จเร็ว แยก git worktree กันไฟล์ชนกัน |

## 4. เครื่องมือและการทำงานร่วมกัน
- **ตรวจด้วยคำสั่ง:** ทุก ticket มี `verify:` ที่เรียก `scripts/verify.py` ซึ่งเปิด backend แล้วรัน contract test ชุดเดียวกับทั้ง A และ B
- **ตรวจงานของ agy:** ticket ไหน agy ทำ Claude รัน verify ซ้ำเองก่อนติ๊กทุกครั้ง
  - รอบ C8-A agy รายงานว่าผ่าน แต่ตอนรันซ้ำเจอเทสต์ AC11 ล้ม เพราะ fixed window คร่อมขอบนาที
- **ติดตามงาน:** Trello เป็นกระจก ส่วน `tickets.md` เป็นตัวจริง

## 5. ผลลัพธ์
- **เทสต์ทั้งสอง backend:**
  - contract 57/57
  - e2e 20/20
  - unit และ integration ของแต่ละ backend (cargo test, go test + testcontainers)
- **security review แบบ adversarial:** ไม่เจอข้อร้ายแรง เจอข้อที่ควรแก้ 4 ข้อที่เทสต์ไม่จับ ทุกข้อเขียน red test ก่อนแก้
- **deploy จริง, Lighthouse, redirect p95:** [TBC]

## 6. ใช้ AI ขั้นไหน และตัดสินใจเองตรงไหน
- **AI ทำ:**
  - ร่างเอกสารทุกขั้น
  - เขียนโค้ด
  - รีวิว security
  - หาเวอร์ชันแพ็กเกจจาก registry
  - วัด CPU บน Workers
- **ตะวันตัดสิน:**
  - ทิศทางดีไซน์
  - ADR-002
  - การแบ่งงานคู่ขนาน
  - trade-off ที่เหลือ เช่นการล็อกบัญชีด้วยการเดาผิด
- **กฎที่ใช้ได้ผล:**
  - agent ห้ามแก้เทสต์และสัญญา API: agy หยุดและรายงานเมื่อเจอ bug ในเทสต์ แทนที่จะแก้เทสต์ให้ผ่าน
  - ผู้ตรวจไม่ใช่ผู้เขียน

## 7. สิ่งที่เรียนรู้ และถ้าทำใหม่จะเปลี่ยน
- **วัดบน production ตั้งแต่ขั้นออกแบบ:** ข้อจำกัดสำคัญของ Workers ไม่โผล่ใน `wrangler dev`
- **ใส่ `.gitattributes` บังคับ LF ตั้งแต่ commit แรก:** บน Windows, CRLF ทำให้เครื่องมือล้ม 3 ครั้ง
- **กำหนดเจ้าของไฟล์ร่วมก่อนทำคู่ขนาน:** `ci.yml` และ `verify.py` ถูกสองฝ่ายแก้แล้วชนกัน
- **เครื่องเดียวรัน Docker, wrangler และ agent หลายตัวพร้อมกันไม่ไหว:** memory ต่ำ และ Docker หยุดเองหลายครั้ง
- **ช่องโหว่ของ skill 24 ข้อ:** จดไว้ใน `scratch/tasks/pilot-shortlink/plan.md` ข้อ 12 ใน TAWAN-OS จะนำกลับไปแก้ `web-build-workflow`

ภาพหน้าจอ: `docs/screenshots/` (desktop, มือถือ 320px, dark mode)
