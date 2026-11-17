# AGENTS.md: pilot-shortlink

ไฟล์นี้คือ context ที่ agent ทุกตัว (Claude Code, agy) อ่านก่อนทำงานในโปรเจกต์นี้ ถ้าขัดกับ `D:\TAWAN-OS\TAWAN-OS-AGENT\AGENTS.md` ให้ยึดของ TAWAN-OS และแจ้งตะวัน

## โปรเจกต์
- ทำอะไร: แอปย่อลิงก์ที่มีหลายบัญชีและสถิติคลิก ทำสองครั้งด้วย backend ต่างกันเพื่อทดสอบ `web-build-workflow`
- ระดับ: 2 | Profile: A (Cloudflare Workers + Rust + D1) และ B (Go บน Render + Neon)
- เอกสาร: `docs/requirements.md` (มีแล้ว), `docs/research.md`, `docs/architecture.md`, `api/openapi.yaml`, `DESIGN.md`, `docs/test-plan.md` (ทำในขั้นถัดไป)
- Ticket: `D:\TAWAN-OS\TAWAN-OS-AGENT\scratch\tasks\pilot-shortlink\tickets.md`
- Trello: ดู `trello.json` ในโฟลเดอร์ ticket

## คำสั่ง
กำหนดในขั้น 3-4 หลังเลือก stack ทุกตัว ระหว่างนี้ยังไม่มีโค้ด

## กฎของโปรเจกต์
- ติดตั้งแพ็กเกจด้วย lockfile เท่านั้น (`pnpm install --frozen-lockfile`)
- type ฝั่ง frontend สร้างจาก `api/openapi.yaml` ห้ามเขียนซ้ำด้วยมือ
- backend A และ B ต้องผ่าน contract และ e2e ชุดเดียวกัน
- ห้ามเก็บ IP หรือ User-Agent ของคนที่คลิก (requirements ข้อ 4)
- ห้าม commit secret, `.env` อยู่ใน `.gitignore`
- ห้ามเพิ่ม dependency โดยไม่บอกเหตุผล

## ห้ามแตะ
- `docs/requirements.md` หลังตะวันยืนยันแล้ว แก้ได้เมื่อตะวันสั่งเท่านั้น
