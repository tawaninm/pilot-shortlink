# AGENTS.md: pilot-shortlink

ไฟล์นี้คือ context ที่ agent ทุกตัว (Claude Code, agy) อ่านก่อนทำงานในโปรเจกต์นี้ ถ้าขัดกับ `D:\TAWAN-OS\TAWAN-OS-AGENT\AGENTS.md` ให้ยึดของ TAWAN-OS และแจ้งตะวัน

## โปรเจกต์
- ทำอะไร: แอปย่อลิงก์ที่มีหลายบัญชีและสถิติคลิก ทำสองครั้งด้วย backend ต่างกันเพื่อทดสอบ `web-build-workflow`
- ระดับ: 2 | Profile: A (Cloudflare Workers + Rust + D1) และ B (Go บน Render + Neon)
- เอกสาร: `docs/requirements.md` (มีแล้ว), `docs/research.md`, `docs/architecture.md`, `api/openapi.yaml`, `DESIGN.md`, `docs/test-plan.md`
- Ticket: `D:\TAWAN-OS\TAWAN-OS-AGENT\scratch\tasks\pilot-shortlink\tickets.md`
- Trello: ดู `trello.json` ในโฟลเดอร์ ticket

## คำสั่ง
```bash
pnpm install --frozen-lockfile
git config core.hooksPath .githooks          # ครั้งเดียวต่อ clone เปิด pre-commit
python scripts/verify.py checks               # tokens + contract typecheck
python scripts/verify.py a [ตัวกรอง vitest]   # backend A ทั้งชุด: clippy, cargo test, D1 migrate, wrangler dev, contract
python scripts/verify.py b [ตัวกรอง vitest]   # backend B: vet, go test, Postgres ใน Docker, migrate, server, contract
pnpm -F contract types                        # สร้าง type ใหม่หลังแก้ api/openapi.yaml
```
ticket แต่ละใบบอกคำสั่ง `verify:` ของตัวเอง ticket ผ่านเมื่อคำสั่งนั้น exit 0 เท่านั้น contract test ใช้ตัวแปร `API_URL` (ไม่ใช่ `BASE_URL` เพราะ Vitest ตั้งค่านี้เอง)

## กฎของโปรเจกต์
- ติดตั้งแพ็กเกจด้วย lockfile เท่านั้น (`pnpm install --frozen-lockfile`)
- type ฝั่ง frontend สร้างจาก `api/openapi.yaml` ห้ามเขียนซ้ำด้วยมือ
- backend A และ B ต้องผ่าน contract และ e2e ชุดเดียวกัน
- ห้ามเก็บ IP หรือ User-Agent ของคนที่คลิก (requirements ข้อ 4)
- ห้าม commit secret, `.env` อยู่ใน `.gitignore`
- ห้ามเพิ่ม dependency โดยไม่บอกเหตุผล

## ห้ามแตะ
- `docs/requirements.md` หลังตะวันยืนยันแล้ว แก้ได้เมื่อตะวันสั่งเท่านั้น
- `api/openapi.yaml`, `contract/test/`, `design/tokens.json` ระหว่างขั้น 5-6 (agy) ถ้าสัญญาหรือเทสต์ผิด ให้หยุดและส่งกลับ Claude Code
