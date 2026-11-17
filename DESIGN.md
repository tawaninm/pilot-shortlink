# DESIGN.md: pilot-shortlink

ทิศทาง A "บัญชีลิงก์" (ตะวันเลือก 2026-11-17) brief และเหตุผลอยู่ที่ `docs/design/brief.md` ค่าทุกค่าอยู่ใน `design/tokens.json` ไฟล์นี้อธิบายวิธีใช้

## หลัก
- **พระเอกของหน้าหลัก:** ฟอร์มวาง URL แถวเดียวบนสุด ตามด้วยตารางลิงก์ หน้าสถิติกางใต้แถวที่เลือก ไม่เปลี่ยนหน้า
- **ข้อมูลแน่นแต่อ่านง่าย:** เห็น 10-12 ลิงก์ต่อจอบน desktop แถวสูง `--size-row` (48px)
- **สีมีหน้าที่:** accent สีเขียวใช้กับลิงก์, ปุ่ม primary, กราฟ และ focus เท่านั้น สีแดงใช้กับ error และการลบเท่านั้น สถานะทุกอย่างมีข้อความกำกับ ไม่ใช้สีอย่างเดียว
- **ชิดซ้าย:** จัดกลางได้เฉพาะหน้า 404 ที่มีข้อความสั้น

## Token
- 3 ชั้น: `primitive.*` (ห้ามใช้ในโค้ด product) → `color.*`, `font.*`, `space.*`, `radius.*`, `size.*`, `focus.*`, `motion.*` (semantic) → `component.*`
- light อยู่ใน `$value` dark อยู่ใน `$extensions.mode.dark` สร้าง CSS ด้วย `node design/build-tokens.mjs` (ห้ามแก้ `design/tokens.css` ด้วยมือ, CI รัน `--check`)
- เปลี่ยนโหมด: ตาม `prefers-color-scheme` และบังคับได้ด้วย `data-theme="light|dark"` บน `<html>`
- contrast ตรวจแล้ว 2026-11-17 ทุกคู่ผ่านทั้งสองโหมด: ข้อความ 4.5:1 ขึ้นไป, `border-strong` และ focus 3:1 ขึ้นไป (`border` 1.3:1 ใช้กับเส้นแบ่งแถวที่เป็นของตกแต่งเท่านั้น ห้ามใช้เป็นขอบ input)
- ดูด้วยตา: เปิด `design/preview.html` ใน browser

## ตัวอักษร
- UI: IBM Plex Sans Thai (400, 500, 600) โหลดจาก Google Fonts พร้อม fallback `system-ui`
- รหัสลิงก์และตัวเลข: IBM Plex Mono (เลขกว้างเท่ากัน คอลัมน์คลิกจึงตรงกัน)
- ขนาด: `sm` 0.875rem, `md` 1rem, `lg` 1.125rem, `xl` 1.5rem ใช้ rem ทั้งหมดเพื่อให้ซูม 200% ได้
- line-height เนื้อหา 1.6 (ภาษาไทยมีวรรณยุกต์ซ้อน ต้องไม่ต่ำกว่า 1.5)

## Layout
- เนื้อหากว้างสุด `--size-content-max` (1040px) ระยะห่างใช้ `space` ฐาน 4px
- radius ตามลำดับชั้น: ตารางและการ์ด `lg` (10px), ปุ่มและ input `md` (6px), badge `sm` (4px)
- มือถือ (ต่ำกว่า 640px): ตารางเปลี่ยนเป็นแถวสองบรรทัด บรรทัดบน `ลิงก์สั้น ... คลิก` บรรทัดล่างปลายทาง กราฟเล็กซ่อน
- กว้าง 320px และซูม 200% ต้องไม่มี scroll แนวนอน ปลายทางยาวไม่มีวรรคตัดด้วย `overflow-wrap: anywhere`

## Component (inventory เริ่มต้น)
| Component | ใช้ที่ไหน | state ที่ต้องมี |
|---|---|---|
| `CreateLinkForm` | บนสุดของหน้าหลัก | default, กำลังส่ง, alias ซ้ำ (409), URL ไม่ถูกต้อง (422), rate limit (429) |
| `LinkTable` / `LinkRow` | หน้าหลัก | กำลังโหลด, ยังไม่มีลิงก์, error, แถวกางสถิติ |
| `LinkStats` | กางใต้แถว | กำลังโหลด, ยังไม่มีคลิก, มีข้อมูล |
| `Sparkline` | คอลัมน์ 30 วัน | ไม่มีคลิกเลยแสดงเส้นแบน |
| `ReferrerBars` | ใน `LinkStats` | มีกลุ่ม "ไม่ทราบที่มา" เสมอเมื่อมี |
| `CopyButton` | ทุกแถว | default, คัดลอกแล้ว (ข้อความ "คัดลอกแล้ว" 2 วินาที) |
| `ConfirmDelete` | ลบลิงก์ | ใช้ `<dialog>` ปุ่มลบสีแดง ปุ่มยกเลิกได้ focus ก่อน |
| `AuthForm` | สมัคร, เข้าสู่ระบบ | ผิดพลาด, email ซ้ำ, กำลังส่ง, ล็อกชั่วคราวจาก rate limit |

ก่อนสร้าง component ใหม่ให้ดูตารางนี้ก่อน ไม่มีของตรงให้หยุดและเสนอ (กฎ `uxui-workflow`)

## ไอคอน
- library เดียว: Phosphor Icons น้ำหนัก regular ขนาด 20px สี `currentColor`
- ใช้เฉพาะ: คัดลอก, ลบ, กางแถว, ออกจากระบบ, ลิงก์ภายนอก ปุ่มที่มีแต่ไอคอนต้องมี `aria-label` ภาษาไทย
- ไม่ใส่ไอคอนนำหน้าช่องกรอก

## Motion
- กางแถวสถิติ `--motion-duration-fast` (150ms) ด้วย `--motion-easing-standard` เท่านั้น ไม่มี animation ตอนโหลดหน้า
- `prefers-reduced-motion: reduce` ปิด transition ทั้งหมด

## ห้าม
ทุกข้อใน `uxui-workflow/references/slop.md` และเฉพาะงานนี้: พื้นครีม, ม่วงหรือ indigo, การ์ดสถิติ 4 ใบเท่ากัน, hero จัดกลาง, รีวิวหรือตัวเลขผู้ใช้ปลอม, ภาพประกอบตกแต่ง
