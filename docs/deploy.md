# Runbook การ Deploy (ขั้น 8): pilot-shortlink

วันที่: 2026-11-17 | ระดับ: 2 | Profile: A (Cloudflare Workers + D1) และ B (Render + Neon) | ข้อจำกัด: Free tier เท่านั้น (`docs/requirements.md` ข้อ 6)

เอกสารนี้คือคู่มือทีละขั้นสำหรับตะวันในการนำระบบขึ้นใช้งานจริง แยกชัดเจนระหว่าง **"ตะวันทำ"** (การ login, สร้างทรัพยากรบนคลาวด์, วางค่า secret ในหน้า Dashboard) กับ **"คำสั่งที่รัน"** ในเทอร์มินัล โดยไม่มีการเก็บค่า secret ลงในไฟล์ของโปรเจกต์

---

## 0. รายการค่าที่ต้องแทนที่ (`[TBC]`)

ก่อนหรือระหว่างทำตามขั้นตอนด้านล่าง ให้แทนที่ค่าที่มีเครื่องหมาย `[TBC]` ทั้งหมดนี้ด้วยค่าจริงจากบัญชีของตะวัน:

| รหัส `[TBC]` | ความหมาย | ที่มาของค่า | ที่ที่ต้องนำไปใส่ |
|---|---|---|---|
| `[TBC: <SUBDOMAIN>]` | ชื่อ subdomain `*.workers.dev` ของบัญชี Cloudflare | หน้า **Workers & Pages** ใน Cloudflare Dashboard | `backend-a/wrangler.toml` (`[env.production.vars]` `APP_ORIGIN`) |
| `[TBC: <D1_DATABASE_ID>]` | UUID ของฐานข้อมูล D1 สำหรับ production | ผลลัพธ์จากคำสั่ง `npx wrangler@4.163.0 d1 create pilot-shortlink-a` | `backend-a/wrangler.toml` (`[[env.production.d1_databases]]` `database_id`) |
| `[TBC: <CF_ACCOUNT_ID>]` | Cloudflare Account ID (เฉพาะกรณีที่บัญชีมีหลาย workspace) | หน้า **Workers & Pages** ด้านขวาของ Cloudflare Dashboard | ตัวแปรสภาพแวดล้อม `CLOUDFLARE_ACCOUNT_ID` ตอนรัน Wrangler (ถ้ามีบัญชีเดียวไม่ต้องตั้ง) |
| `[TBC: <NEON_DIRECT_DATABASE_URL>]` | Connection string ของ Neon แบบ **Direct** (ไม่มี `-pooler`, มี `?sslmode=require`) | ปุ่ม **Connect** ใน Neon Console (ปิดสวิตช์ Connection pooling) | ตัวแปรสภาพแวดล้อมชั่วคราวในเครื่องตะวันตอนรัน `/app/migrate up` หรือ `down` (ห้ามเขียนลงไฟล์ใน git) |
| `[TBC: <NEON_POOLED_DATABASE_URL>]` | Connection string ของ Neon แบบ **Pooled** (มี `-pooler` ใน host, มี `?sslmode=require`) | ปุ่ม **Connect** ใน Neon Console (เปิดสวิตช์ Connection pooling) | ตัวแปร `DATABASE_URL` บน Render Dashboard ตอนสร้าง Blueprint |
| `[TBC: <RENDER_SERVICE_URL>]` | URL จริงของ Web Service บน Render (เช่น `https://pilot-shortlink-b.onrender.com` หรือ `https://pilot-shortlink-b-xxxx.onrender.com` **ห้ามมี `/` ปิดท้าย**) | มุมบนซ้ายของหน้าบริการใน Render Dashboard หลังสร้างเสร็จ | ตัวแปร `APP_ORIGIN` บน Render Dashboard |
| `[TBC: <GITHUB_REPO_URL>]` | URL ของ GitHub repository ที่เก็บโค้ดโปรเจกต์นี้ | GitHub ของตะวัน | หน้าเชื่อมต่อ Blueprint ใน Render Dashboard |
| `[TBC: Error Tracking]` | บริการ Error Tracking ภายนอก (เช่น Sentry ตาม `docs/architecture.md` ข้อ 8) | บัญชี Sentry ของตะวัน (ถ้ายังไม่ตั้งในรอบ pilot ให้ใช้ Workers Logs ของ A และ Render Logs ของ B แทน และบันทึกเป็นข้อจำกัดของ pilot) | บันทึกในรายงานปิดงาน |

---

## 1. การ Deploy Backend A (Cloudflare Workers + Rust + D1 — Free Plan)

### ขั้น 1.1: เข้าสู่ระบบ Cloudflare และตรวจสอบ `workers.dev` Subdomain

**ตะวันทำ:**
1. เปิดเบราว์เซอร์เข้า [Cloudflare Dashboard](https://dash.cloudflare.com/) แล้วไปที่เมนู **Workers & Pages**
2. ดูชื่อ subdomain `*.workers.dev` ในแถบด้านขวา (นี่คือค่า `[TBC: <SUBDOMAIN>]`) หากยังไม่เคยตั้งชื่อ subdomain มาก่อน ให้กดตั้งค่าครั้งเดียวในหน้า Dashboard
3. รันคำสั่ง login ด้านล่าง แล้วกด **Allow** ในหน้าต่างเบราว์เซอร์ที่เด้งขึ้นมา

**คำสั่งที่รัน:**
```bash
npx --yes wrangler@4.163.0 login
npx --yes wrangler@4.163.0 whoami
```

### ขั้น 1.2: สร้างฐานข้อมูล D1 และใส่ค่าใน `backend-a/wrangler.toml`

**คำสั่งที่รัน:**
```bash
cd backend-a
npx --yes wrangler@4.163.0 d1 create pilot-shortlink-a
```

**ตะวันทำ:**
1. คัดลอกค่า `database_id` (UUID) ที่แสดงในเทอร์มินัลหลังรันคำสั่ง (`[TBC: <D1_DATABASE_ID>]`)
2. เปิดไฟล์ `backend-a/wrangler.toml` แล้วแก้ไขส่วน `[env.production]`:
   - แทนที่ `<D1_DATABASE_ID>` ใน `[[env.production.d1_databases]]` ด้วย UUID จริงที่ได้มา
   - แทนที่ `<SUBDOMAIN>` ใน `[env.production.vars]` `APP_ORIGIN = "https://pilot-shortlink-a.<SUBDOMAIN>.workers.dev"` ด้วย subdomain จริงของบัญชี (**ห้ามใส่ `/` ปิดท้าย** เพราะเซิร์ฟเวอร์ตรวจ header `Origin` แบบตรงตัวทุกตัวอักษร)

### ขั้น 1.3: รัน D1 Migration บนฐานข้อมูลจริง (`--remote`)

**คำสั่งที่รัน:**
```bash
cd backend-a
npx --yes wrangler@4.163.0 d1 migrations list DB --env production --remote
npx --yes wrangler@4.163.0 d1 migrations apply DB --env production --remote
```

**ตะวันทำ:**
- พิมพ์ `y` เมื่อ Wrangler ถามยืนยันการรันไฟล์ `0001_init.sql` บนฐานข้อมูล remote

**คำสั่งที่รัน (ตรวจโครงสร้างตารางหลัง migrate ตาม AC9 ว่าไม่มีคอลัมน์ IP หรือ User-Agent):**
```bash
npx --yes wrangler@4.163.0 d1 execute DB --env production --remote --command "PRAGMA table_info(click_daily);"
```

### ขั้น 1.4: Build Frontend และ Deploy Worker ขึ้น Production

ตาม ADR-001 ทั้ง frontend และ API เสิร์ฟจาก origin เดียวกันผ่าน binding `ASSETS` (`../web/dist`) จึงต้อง build frontend ก่อน deploy ทุกครั้ง

**คำสั่งที่รัน (จากรากของ repo `D:\TAWAN-OS\pilot-shortlink`):**
```bash
pnpm -F web build
cd backend-a
npx --yes wrangler@4.163.0 deploy --env production --dry-run
npx --yes wrangler@4.163.0 deploy --env production
```

**ตะวันทำ:**
- ตรวจสอบ URL ที่ Wrangler แสดงเมื่อ deploy เสร็จว่าตรงกับค่า `APP_ORIGIN` ใน `backend-a/wrangler.toml` ทุกตัวอักษร หากไม่ตรง ให้แก้ `APP_ORIGIN` ให้ตรงแล้วรัน `npx --yes wrangler@4.163.0 deploy --env production` ซ้ำอีกครั้ง

---

## 2. การ Deploy Backend B (Go บน Render Free + Neon Postgres Free)

> **ข้อจำกัดของ Render Free Plan สำหรับการรัน Migration:**
> - เอกสารทางการของ Render ระบุว่า `preDeployCommand` รองรับเฉพาะแผนแบบเสียเงิน (Paid compute plans) เท่านั้น ไม่รองรับบน `plan: free` หากใส่ `preDeployCommand` ใน `render.yaml` ของบริการแบบ `free` จะเกิดข้อผิดพลาดตอนตรวจสอบ Blueprint ([Render Deploys: Pre-Deploy Command](https://render.com/docs/deploys#pre-deploy-command), [Blueprint Spec: preDeployCommand](https://render.com/docs/blueprint-spec#predeploycommand))
> - Render Free ไม่รองรับการเข้าใช้งาน SSH / Web Shell และไม่รองรับ One-Off Jobs ([Render Free Instance Limitations](https://render.com/docs/free), [Render Jobs](https://render.com/docs/jobs))
> - นอกจากนี้ `backend-b/Dockerfile` ใช้ base image แบบ `gcr.io/distroless/static-debian13:nonroot` ซึ่งไม่มี shell แต่คอมไพล์ไบนารี `/app/migrate` ไว้คู่กับ `/app/server`
> - ดังนั้น **ต้องรัน `/app/migrate up` แบบ manual จากเครื่องของตะวันชี้ไปที่ฐานข้อมูล Neon ก่อนเริ่มบริการครั้งแรก และก่อนการ deploy รอบที่มีการเปลี่ยน schema ทุกครั้ง**

### ขั้น 2.1: สร้างโปรเจกต์และฐานข้อมูลบน Neon (Free Plan)

**ตะวันทำ:**
1. เข้าสู่ระบบที่ [Neon Console](https://console.neon.tech/) แล้วกด **New Project**
   - ตั้งชื่อโปรเจกต์ เช่น `pilot-shortlink`
   - เลือก Region ที่ใกล้กับ Region ของ Render ที่จะใช้ (เช่น Singapore หรือ US East)
2. ในหน้า Dashboard ของโปรเจกต์ กดปุ่ม **Connect** แล้วคัดลอก Connection String เก็บไว้ 2 แบบ (ทั้งคู่ต้องมีพารามิเตอร์ `?sslmode=require` เสมอ และห้ามนำไปเขียนลงไฟล์ใน git):
   - **Direct connection string** (ปิดสวิตช์ **Connection pooling** — ชื่อ host ไม่มี `-pooler`): ใช้เป็นค่า `[TBC: <NEON_DIRECT_DATABASE_URL>]` สำหรับรันคำสั่ง migrate
   - **Pooled connection string** (เปิดสวิตช์ **Connection pooling** — ชื่อ host มี `-pooler`): ใช้เป็นค่า `[TBC: <NEON_POOLED_DATABASE_URL>]` สำหรับใส่ใน `DATABASE_URL` บน Render Dashboard

### ขั้น 2.2: รัน Migration (`/app/migrate up`) เข้าฐานข้อมูล Neon แบบ Manual

เลือกรันวิธีใดวิธีหนึ่งด้านล่างจากเครื่องของตะวัน โดยใช้ค่า `[TBC: <NEON_DIRECT_DATABASE_URL>]`:

**คำสั่งที่รัน — วิธีที่ 1 (ใช้ไบนารี `/app/migrate` ใน Docker image เดียวกับที่ Render ใช้จริง):**
```powershell
# รันจากรากของ repo (D:\TAWAN-OS\pilot-shortlink)
docker build -f backend-b/Dockerfile -t pilot-shortlink-b .
docker run --rm --entrypoint /app/migrate -e DATABASE_URL="[TBC: <NEON_DIRECT_DATABASE_URL>]" pilot-shortlink-b up
docker run --rm --entrypoint /app/migrate -e DATABASE_URL="[TBC: <NEON_DIRECT_DATABASE_URL>]" pilot-shortlink-b status
```

**คำสั่งที่รัน — วิธีที่ 2 (รันผ่าน `go run ./cmd/migrate` จากโฟลเดอร์ `backend-b/`):**
```powershell
cd backend-b
$env:DATABASE_URL="[TBC: <NEON_DIRECT_DATABASE_URL>]"
go run ./cmd/migrate up
go run ./cmd/migrate status
Remove-Item Env:\DATABASE_URL
```

### ขั้น 2.3: เชื่อมต่อ Render Blueprint (`render.yaml`) และตั้งค่า Environment Variables

**คำสั่งที่รัน (นำไฟล์ขึ้น GitHub เพื่อให้ Render อ่าน `render.yaml`):**
```bash
git push origin main
```

**ตะวันทำ:**
1. เข้าสู่ระบบที่ [Render Dashboard](https://dashboard.render.com/) -> กดปุ่ม **New +** -> เลือก **Blueprint**
2. เชื่อมต่อกับ GitHub repository `[TBC: <GITHUB_REPO_URL>]` และเลือก branch ที่ต้องการ deploy
3. Render จะตรวจพบไฟล์ `render.yaml` ที่รากของ repo และแสดงช่องให้กรอกตัวแปรสภาพแวดล้อมที่กำหนด `sync: false` ไว้ 2 ตัว:
   - `DATABASE_URL`: วางค่า `[TBC: <NEON_POOLED_DATABASE_URL>]` (Pooled connection string จาก Neon ที่มี `?sslmode=require`)
   - `APP_ORIGIN`: ใส่ URL ของบริการ เช่น `https://pilot-shortlink-b.onrender.com` (`[TBC: <RENDER_SERVICE_URL>]` **ห้ามมี `/` ปิดท้าย**)
4. กด **Apply** เพื่อสร้างบริการและเริ่ม build Docker image (เมื่อพร้อมแล้ว Render จะตรวจ `/health` อัตโนมัติตาม `healthCheckPath: /health`)
5. **ตรวจสอบ URL จริงหลังสร้างบริการเสร็จ:**
   - ดู URL จริงที่มุมบนซ้ายของหน้าบริการ `pilot-shortlink-b` ใน Render Dashboard
   - หาก Render เติมรหัสสุ่มต่อท้ายชื่อบริการ (เช่น `https://pilot-shortlink-b-xxxx.onrender.com`) ให้ไปที่แท็บ **Environment** แก้ค่า `APP_ORIGIN` ให้ตรงกับ URL จริงทุกตัวอักษร (ไม่มี `/` ปิดท้าย) แล้วกด **Save, rebuild, and deploy** (หากไม่ตรง คำขอ `POST`, `PATCH`, `DELETE` จะถูกปฏิเสธด้วย `403 forbidden_origin`)

---

## 3. การทดสอบหลัง Deploy (Smoke Test ด้วย `curl`)

`python scripts/verify.py` ใช้สำหรับรันทดสอบกับเซิร์ฟเวอร์ในเครื่อง (local) เท่านั้น หลัง deploy ขึ้นคลาวด์แล้ว ให้รันชุดคำสั่งด้านล่างกับทั้ง **Backend A** (`https://pilot-shortlink-a.<SUBDOMAIN>.workers.dev`) และ **Backend B** (`[TBC: <RENDER_SERVICE_URL>]`)

> **หมายเหตุสำหรับ Backend B (Cold Start):** บนแผน Free ทั้ง Render (พักหลังไม่มี traffic 15 นาที) และ Neon (พักหลังไม่ใช้งาน 5 นาที) จะเข้าสู่โหมดพักอัตโนมัติ คำขอแรกหลังพักอาจใช้เวลาปลุกเครื่องราว 30–60 วินาที

### ขั้น 3.1: ตั้งค่าตัวแปร URL เป้าหมาย (รันใน Git Bash / WSL / bash)

```bash
# เปลี่ยนเป็น URL จริงของ A หรือ B (ไม่มี / ปิดท้าย)
export APP_URL="[TBC: https://pilot-shortlink-a.<SUBDOMAIN>.workers.dev หรือ <RENDER_SERVICE_URL>]"
```

### ขั้น 3.2: ตรวจ `/health` และ Header `Content-Security-Policy` บนหน้า `/`

**คำสั่งที่รัน:**
```bash
# 1. ตรวจ /health: ต้องได้ HTTP/2 200 และ body {"status":"ok"}
curl -i "$APP_URL/health"

# 2. ตรวจ CSP และ Security Headers บนหน้า /:
# ต้องเห็น Content-Security-Policy: default-src 'self'; ...
# X-Content-Type-Options: nosniff และ Referrer-Policy: strict-origin-when-cross-origin
curl -i -sS "$APP_URL/" | grep -iE "^(http/|content-security-policy:|x-content-type-options:|referrer-policy:)"
```

### ขั้น 3.3: ตรวจการสมัครสมาชิก + สร้างลิงก์สั้น + เปิดลิงก์ Redirect (302)

คำขอที่เปลี่ยนข้อมูล (`POST`, `PATCH`, `DELETE`) ต้องส่ง header `Origin: $APP_URL` ให้ตรงกับ `APP_ORIGIN` เสมอ:

**คำสั่งที่รัน:**
```bash
# 1. สมัครสมาชิก (ต้องได้ HTTP 201 พร้อม Set-Cookie: session=...; HttpOnly; Secure; SameSite=Lax)
curl -i -c smoke-cookies.txt \
  -X POST "$APP_URL/api/auth/register" \
  -H "Origin: $APP_URL" \
  -H "Content-Type: application/json" \
  -d '{"email":"smoke-test@example.com","password":"smoke-password-123"}'

# 2. สร้างลิงก์สั้นพร้อม alias (ต้องได้ HTTP 201 และคืน JSON ของลิงก์ "smoke-cv")
curl -i -b smoke-cookies.txt \
  -X POST "$APP_URL/api/links" \
  -H "Origin: $APP_URL" \
  -H "Content-Type: application/json" \
  -d '{"url":"https://example.com/cv","alias":"smoke-cv"}'

# 3. เปิดลิงก์สั้นเพื่อตรวจ Redirect (ต้องได้ HTTP 302, Location: https://example.com/cv และ Cache-Control: no-store)
curl -i "$APP_URL/smoke-cv"

# 4. ตรวจสถิติของลิงก์ (ต้องได้ HTTP 200 และ clicksTotal เป็น 1)
curl -i -b smoke-cookies.txt "$APP_URL/api/links/smoke-cv/stats"

# 5. ลบลิงก์ทดสอบ ออกจากระบบ และลบไฟล์ cookie ชั่วคราว
curl -i -b smoke-cookies.txt -X DELETE "$APP_URL/api/links/smoke-cv" -H "Origin: $APP_URL"
curl -i -b smoke-cookies.txt -X POST "$APP_URL/api/auth/logout" -H "Origin: $APP_URL"
rm -f smoke-cookies.txt
```

### ขั้น 3.4: (ทางเลือก) รันชุดทดสอบ Playwright E2E กับ URL จริง (AC12)

**คำสั่งที่รัน:**
```bash
E2E_URL="$APP_URL" pnpm -F e2e test
```

---

## 3.5 รอบ pilot-addons (ยืนยันอีเมล, รีเซ็ตรหัสผ่าน, QR) deploy 2026-11-19

ลำดับที่ใช้จริง (migration ทั้งสองฝั่งเพิ่มคอลัมน์และตารางเท่านั้น โค้ดเก่ารันกับ schema ใหม่ได้ แต่โค้ดใหม่ต้องมี schema ใหม่ก่อน จึง migrate ก่อน deploy เสมอ):
1. Secret: `RESEND_API_KEY` ตั้งเองโดยตะวัน (`npx --yes wrangler@4.163.0 secret put RESEND_API_KEY --env production` ใน `backend-a` และตัวแปรใน Render Dashboard) `MAIL_FROM` ไม่ต้องตั้ง (ใช้ `onboarding@resend.dev` ส่งได้เฉพาะอีเมลเจ้าของบัญชี Resend) ห้ามตั้ง `MAIL_MODE` บน production
2. A: `npx --yes wrangler@4.163.0 d1 migrations apply DB --env production --remote` (0002_email_tokens.sql) แล้ว `pnpm -F web build` และ `wrangler deploy --env production`
3. B: ตะวันรัน `go run ./cmd/migrate up` ใน `backend-b` ด้วย Neon Direct URL (ใส่ผ่าน `Read-Host` แล้ว `Remove-Item Env:\DATABASE_URL`) ต้องเห็น `applied 00002_email_tokens.sql` แล้วจึง `git push origin main` ให้ Render deploy
4. Smoke ทั้งสอง URL: `/api/test/mail?to=x@example.com` ได้ 404 (AC-V8), `/verify-email` `/forgot-password` `/reset-password` ได้ 200, register ได้ `emailVerified:false`, `POST /api/links` ได้ 403 `email_unverified`, forgot ได้ 204 ใช้อีเมลทดสอบ `delivered+<label>@resend.dev` ไม่ใช้อีเมลคนจริง ดูว่าส่งเมลจริงได้ด้วย `wrangler tail --env production` ต้องเห็น `"event":"mail_send","result":"ok"`
5. ตะวันทดสอบเอง: สมัครด้วยอีเมลเจ้าของบัญชี Resend กดลิงก์ยืนยัน, ลืมรหัสผ่านแล้วตั้งใหม่ (AC-V12), สแกน QR ด้วยมือถือ

หมายเหตุ: `curl -I` (HEAD) ที่ `/` ของ B ได้ 405 มาตั้งแต่ก่อนรอบนี้ ตรวจ header ด้วย GET (`curl -s -D - -o /dev/null <URL>/`)

## 3.6 รอบ pilot-oauth (Login ด้วย Google/GitHub และ Turnstile) deploy 2026-11-19

ลำดับที่ใช้จริง (migration เพิ่มตารางอย่างเดียว โค้ดเก่ารันกับ schema ใหม่ได้ แต่ต้อง migrate ก่อน deploy เสมอ):
1. **สมัครบริการ (ตะวันทำ):** Google Cloud > Google Auth Platform: OAuth client แบบ Web application ตัวเดียวใส่ redirect URI 2 อัน (A และ B `/api/auth/oauth/google/callback`) สถานะ Testing ขอ scope `openid email profile` และเพิ่มอีเมลผู้ทดสอบใน Audience > Test users; GitHub > Developer settings > OAuth Apps: แอปเดียวใส่ redirect URI 2 อัน (GitHub ให้ได้สูงสุด 10) ปิด wildcard และ Device Flow; Cloudflare > Turnstile: widget Managed ตัวเดียวใส่ hostname ของ A และ B
2. **Secret (ตะวันตั้งเอง):** `GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET`, `GITHUB_CLIENT_ID`, `GITHUB_CLIENT_SECRET`, `TURNSTILE_SECRET` ที่ A ด้วย `npx --yes wrangler@4.163.0 secret put <ชื่อ> --env production` และที่ B ในหน้า Environment ของ Render (เลือก **Save only** ก่อน) ต้องตั้งให้ครบก่อน deploy เพราะถ้าขาด `TURNSTILE_SECRET` การสมัครและลืมรหัสผ่านจะปฏิเสธทุกครั้ง (fail closed)
3. **ตัวแปรสาธารณะ:** `TURNSTILE_SITE_KEY` ใน `[env.production.vars]` ของ A (`backend-a/wrangler.toml`) และบน Render ต้องเพิ่มเองใน Environment ด้วย เพราะ Render ไม่ดึงค่าจาก `render.yaml` เข้าบริการที่มีอยู่แล้ว (ตอน deploy ครั้งแรก `/api/config` ของ B ตอบ `turnstileSiteKey: null` จนเพิ่มเอง) และ `TRUSTED_PROXY_HOPS` (ดูข้อ 6)
4. **A:** `wrangler d1 migrations apply DB --env production --remote` (0003_oauth.sql) แล้ว `pnpm -F web build` และ `wrangler deploy --env production`
5. **B:** ตะวันรัน `go run ./cmd/migrate up` ใน `backend-b` ด้วย Neon Direct URL (00003_oauth.sql) แล้วจึง `git push origin main` ให้ Render deploy
6. **`TRUSTED_PROXY_HOPS` บน Render = 2:** วัดจริง 2026-11-19 ด้วยการยิง `/api/auth/oauth/google/start` 40 ครั้งโดยปลอม `X-Forwarded-For` ต่างกันทุกครั้ง: ค่า 1 ปล่อยผ่าน 36 ครั้ง (กุญแจเป็น proxy ที่เปลี่ยนไปมา ไม่ใช่ลูกค้า), ค่า 2 ปล่อยผ่าน 31 ครั้งแล้วบล็อก (กุญแจเป็นที่อยู่ลูกค้า และปลอมค่าซ้ายไม่ได้ผล) ถ้าไม่มีครั้งไหนถูกบล็อก แปลว่าค่านั้นเชื่อค่าที่ผู้ใช้ปลอมได้ ให้ลดค่าลงทันที ทดสอบซ้ำทุกครั้งที่เปลี่ยนโครงสร้างหน้าบ้านของ Render ค่าตั้งต้นของโค้ดคือ 0 (ไม่เชื่อ header นี้)
7. **Smoke ทั้งสอง URL:** `/api/test/mail` ได้ 404; `/api/config` ได้ site key; `/api/auth/oauth/google/start` ได้ 302 ไป `https://accounts.google.com/` (ไม่ใช่ localhost) พร้อม cookie `__Host-oauth` (`HttpOnly; Secure; SameSite=Lax`) และ `redirect_uri` ตรงกับ URL ของฝั่งนั้น; provider ที่ไม่รู้จักได้ 404; สมัครโดยไม่มี captcha และด้วย `fake-pass` ได้ 403 `captcha_failed` (พิสูจน์ว่าโหมดทดสอบปิด); CSP เพิ่มเฉพาะ `https://challenges.cloudflare.com` ใน `script-src` และ `frame-src`
8. **ตะวันทดสอบเอง:** กดเข้าสู่ระบบด้วย Google และ GitHub บน A และ B (ผ่านทั้งหมด 2026-11-19), widget Turnstile ขึ้นและยืนยันได้ในหน้าสมัครและลืมรหัสผ่าน

หมายเหตุ: แอป Google สถานะ Testing ให้เฉพาะอีเมลใน Test users เข้าได้ และ Resend แบบไม่มีโดเมนส่งอีเมลได้เฉพาะถึงเจ้าของบัญชี Resend

## 4. การเฝ้าดูตาม ADR-002 (Error 1102 บน `/api/auth/*` ของ Backend A)

ตาม `docs/architecture.md` (ADR-002) Backend A ใช้ Argon2id (`m=19456 KiB, t=2, p=1` ตามเกณฑ์ OWASP) บน Cloudflare Workers Free ซึ่งกำหนดเพดาน CPU ไว้ที่ 10 ms ต่อคำขอ (ผลวัดจริงใน `spikes/RESULTS.md` ใช้ CPU ราว 54 ms median เฉพาะตอนสมัครและเข้าสู่ระบบ)

โค้ดของ Backend A (`backend-a/src/lib.rs`) บันทึก JSON log หนึ่งบรรทัดต่อคำขอ พร้อมเวลา `hash_ms` สำหรับ `/api/auth/register` และ `/api/auth/login` เช่น:
```json
{"method":"POST","path":"/api/auth/login","status":200,"ms":60,"hash_ms":54}
```

### วิธีตรวจและเฝ้าดู

**คำสั่งที่รัน (ดู log สดจากเทอร์มินัลระหว่างทดสอบสมัคร/เข้าสู่ระบบ):**
```bash
cd backend-a
npx --yes wrangler@4.163.0 tail --env production
```

**ตะวันทำ (ตรวจย้อนหลังใน Cloudflare Dashboard):**
1. เข้า [Cloudflare Dashboard](https://dash.cloudflare.com/) -> **Workers & Pages** -> เลือก `pilot-shortlink-a` -> แท็บ **Logs** (เปิดใช้งานไว้แล้วผ่าน `[env.production.observability] enabled = true`)
2. กรองดูคำขอที่ `path` เป็น `/api/auth/register` หรือ `/api/auth/login` และตรวจดูค่า `outcome` หรือข้อความผิดพลาด **Error 1102 (`Worker exceeded CPU time limit`)**
3. **เกณฑ์ตัดสินใจและแผนสำรองตาม ADR-002:**
   - หากพบ **Error 1102** บน `/api/auth/*` ให้ถือเป็น incident ทันที
   - **แผนสำรอง:** เปลี่ยนแผนของ Workers ใน Cloudflare Dashboard เป็น **Workers Paid** ($5/เดือน ได้เพดาน CPU 30 วินาทีต่อคำขอ) โดย**ไม่ต้องแก้ไขโค้ดหรือ deploy ใหม่**

---

## 5. การ Rollback เมื่อเกิดปัญหา

### 5.1 การ Rollback ของ Backend A (Cloudflare Workers + D1)

#### ก. ย้อนกลับโค้ดของ Worker
**คำสั่งที่รัน:**
```bash
cd backend-a
# แสดงรายการ deployment ล่าสุดเพื่อดู Version ID ก่อนหน้า
npx --yes wrangler@4.163.0 deployments list --env production

# ย้อนกลับไปยัง deployment ก่อนหน้าทันที (หรือระบุ Version ID ที่ต้องการ)
npx --yes wrangler@4.163.0 rollback --env production
```

**ตะวันทำ (ทางเลือกผ่านหน้าเว็บ):**
- เข้า **Workers & Pages** -> `pilot-shortlink-a` -> แท็บ **Deployments** -> กดเมนูจุดสามจุดหลังเวอร์ชันก่อนหน้า -> เลือก **Rollback**

#### ข. การกู้คืนฐานข้อมูล D1 (D1 Time Travel)
- **ข้อควรระวัง:** คำสั่ง `wrangler rollback` ย้อนเฉพาะโค้ดและค่าตั้งต้นของ Worker เท่านั้น **ไม่ได้ย้อนข้อมูลหรือ schema ในฐานข้อมูล D1 อัตโนมัติ** ([Workers Rollbacks](https://developers.cloudflare.com/workers/versions-and-deployments/rollbacks/))
- หากการเปลี่ยนแปลงทำให้ข้อมูลหรือ schema ใน D1 เสียหาย ให้กู้คืนฐานข้อมูลด้วย **D1 Time Travel** (ย้อนหลังได้ถึง 7 วันบนแผน Free):

**คำสั่งที่รัน:**
```bash
cd backend-a
# ดู bookmark ณ เวลาก่อนเกิดปัญหา (เวลามาตรฐาน UTC รูปแบบ ISO-8601)
npx --yes wrangler@4.163.0 d1 time-travel info pilot-shortlink-a --env production --timestamp="2026-11-17T00:00:00Z"

# กู้คืนฐานข้อมูลกลับไปยังเวลานั้น
npx --yes wrangler@4.163.0 d1 time-travel restore pilot-shortlink-a --env production --timestamp="2026-11-17T00:00:00Z"
```

### 5.2 การ Rollback ของ Backend B (Render + Neon Postgres)

#### ก. ย้อนกลับ Container บน Render
- บริการบน Render Free Plan รองรับการกด Rollback ไปยัง **2 deploy ล่าสุดก่อนหน้า** ([Render Rollbacks](https://render.com/docs/rollbacks))

**ตะวันทำ:**
1. เข้า [Render Dashboard](https://dashboard.render.com/) -> เลือกบริการ `pilot-shortlink-b` -> แท็บ **Events** หรือ **Deploys**
2. เลือก deploy ก่อนหน้าที่ทำงานได้ปกติ (ภายใน 2 ครั้งล่าสุดบนแผน Free) แล้วกด **Rollback to this deploy**
3. หากต้องการย้อนกลับไปเก่ากว่า 2 deploy ล่าสุดบนแผน Free ให้รัน `git revert <COMMIT_SHA>` แล้ว `git push origin main` เพื่อให้ Render สร้าง deploy ใหม่จากโค้ดที่ย้อนกลับแล้ว

#### ข. ย้อนกลับฐานข้อมูล Neon Postgres
1. **ย้อน Migration ทีละขั้นด้วย `/app/migrate down` (ใช้ส่วน `-- +goose Down` ในไฟล์ SQL):**

   **คำสั่งที่รัน:**
   ```powershell
   docker run --rm --entrypoint /app/migrate -e DATABASE_URL="[TBC: <NEON_DIRECT_DATABASE_URL>]" pilot-shortlink-b down
   ```
   *(หรือรันจาก `backend-b/`: `$env:DATABASE_URL="[TBC: <NEON_DIRECT_DATABASE_URL>]"; go run ./cmd/migrate down; Remove-Item Env:\DATABASE_URL`)*

2. **กู้คืนทั้งข้อมูลและ Schema ด้วย Neon Instant Restore (กรณีข้อมูลเสียหาย):**
   - **ตะวันทำ:** เข้า [Neon Console](https://console.neon.tech/) -> เมนู **Branches** -> เลือก branch หลัก (`main`) -> กด **Restore** เพื่อย้อนฐานข้อมูลกลับไปยังช่วงเวลาก่อนเกิดปัญหา (ภายในหน้าต่าง History retention สูงสุด 24 ชั่วโมงของแผน Free)

---

## 6. เอกสารทางการที่ใช้อ้างอิง (Official Documentation)

1. **Cloudflare Workers & Wrangler:**
   - การตั้งค่า `wrangler.toml`: <https://developers.cloudflare.com/workers/wrangler/configuration/>
   - การแยก Environments (`[env.production]` และการไม่สืบทอด bindings อัตโนมัติ): <https://developers.cloudflare.com/workers/wrangler/environments/>
   - การ Rollback เวอร์ชันของ Worker และข้อจำกัดเรื่องข้อมูล: <https://developers.cloudflare.com/workers/versions-and-deployments/rollbacks/>
   - เพดาน CPU ของ Workers Free (10 ms และ Error 1102): <https://developers.cloudflare.com/workers/platform/limits/>
   - Workers Logs (`[observability]`): <https://developers.cloudflare.com/workers/observability/logs/workers-logs/>
2. **Cloudflare D1:**
   - คำสั่ง Wrangler สำหรับ D1 (`d1 create`, `d1 migrations apply --remote`, `d1 execute`): <https://developers.cloudflare.com/d1/wrangler-commands/>
   - ระบบ Migrations ของ D1: <https://developers.cloudflare.com/d1/reference/migrations/>
   - การกู้คืนข้อมูลด้วย D1 Time Travel: <https://developers.cloudflare.com/d1/reference/time-travel/>
3. **Render:**
   - โครงสร้างไฟล์ Blueprint (`render.yaml`, `runtime: docker`, `dockerfilePath`, `dockerContext`, `healthCheckPath`, `sync: false`): <https://render.com/docs/blueprint-spec>
   - ข้อจำกัดของ `preDeployCommand` (รองรับเฉพาะ Paid plans เท่านั้น): <https://render.com/docs/deploys#pre-deploy-command> และ <https://render.com/docs/blueprint-spec#predeploycommand>
   - ข้อจำกัดของแผน Render Free (ไม่มี SSH/Shell, ไม่มี One-Off Jobs, พักหลังว่าง 15 นาที): <https://render.com/docs/free> และ <https://render.com/docs/jobs>
   - การ Rollback บน Render (แผน Free ย้อนได้ 2 deploy ล่าสุด): <https://render.com/docs/rollbacks>
4. **Neon Postgres:**
   - การใช้ Connection Pooling (`-pooler`) คู่กับ Direct Connection สำหรับ Migration: <https://neon.com/docs/connect/connection-pooling>
   - การเชื่อมต่ออย่างปลอดภัยด้วย `sslmode=require`: <https://neon.com/docs/connect/connect-securely>
   - การกู้คืนฐานข้อมูลย้อนหลัง (Instant Restore): <https://neon.com/docs/introduction/branch-restore>
