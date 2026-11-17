# Page: รายการลิงก์ (หน้าหลัก)

## Purpose
สร้างลิงก์สั้นและเห็นทุกลิงก์ของตัวเองพร้อมจำนวนคลิกในหน้าเดียว

## Layout
บนลงล่าง กว้างสุด `size-content-max` ชิดซ้าย ระยะขอบ `space-4` (มือถือ) และ `space-8` (desktop)
1. header: ชื่อแอปซ้าย, email ผู้ใช้ + ปุ่มออกจากระบบขวา, ห่างจากส่วนถัดไป `space-6`
2. `CreateLinkForm`: label "URL ปลายทาง", ช่อง URL ยืดเต็ม, ช่อง alias, ปุ่ม "ย่อลิงก์" อยู่แถวเดียวบน desktop ห่างกัน `space-2` ข้อความ error ใต้ช่องที่เกี่ยวข้อง
3. หัวข้อ "ลิงก์ของฉัน (N)" ห่างฟอร์ม `space-8`
4. `LinkTable` คอลัมน์: ลิงก์สั้น (mono), ปลายทาง (`color-text-muted`), 30 วัน (`Sparkline`), คลิก (mono ชิดขวา), การกระทำ (`CopyButton`, ปุ่มลบ) แถวสูง `component-table-row-height`

## Components
`CreateLinkForm`, `LinkTable`, `LinkRow`, `LinkStats`, `Sparkline`, `ReferrerBars`, `CopyButton`, `ConfirmDelete` ปุ่ม primary 1 ปุ่มคือ "ย่อลิงก์"

## States
ตาม `docs/design/flows.md` ส่วน Page N3 และ N4: idle, empty, loading (skeleton 5 แถว), error พร้อมลองใหม่, สถิติกาง loading / empty / error / partial

## Responsive
mobile (ต่ำกว่า 640px): ฟอร์มเรียงแนวตั้ง ปุ่มเต็มความกว้าง, ตารางเป็นแถวสองบรรทัด (บน: ลิงก์สั้น + คลิก, ล่าง: ปลายทาง) ซ่อน `Sparkline` ปุ่มการกระทำอยู่ในสถิติที่กางออก
desktop: ตามโครงด้านบน

## ที่มา
`docs/design/brief.md` ทิศทาง A และ `design/preview.html` (2026-11-17)
