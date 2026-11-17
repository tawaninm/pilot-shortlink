package api

import (
	"errors"
	"net/http"
	"strconv"
	"strings"
	"time"

	"github.com/go-chi/chi/v5"
	"github.com/jackc/pgx/v5"
	"github.com/tawaninm/pilot-shortlink/backend-b/internal/db/gen"
)

const (
	statsDays    = 30
	createPerMin = 30
	codeAttempts = 5
	createWindow = time.Minute
	dateLayout   = "2006-01-02"
)

type linkSummary struct {
	Code        string  `json:"code"`
	URL         string  `json:"url"`
	CreatedAt   string  `json:"createdAt"`
	ClicksTotal int64   `json:"clicksTotal"`
	Last30Days  []int64 `json:"last30Days"`
}

// days returns the first UTC day of the 30-day window and a function mapping a day to its slot (or -1).
func (s *server) days() (time.Time, func(time.Time) int) {
	today := s.Now().UTC().Truncate(24 * time.Hour)
	first := today.AddDate(0, 0, -(statsDays - 1))
	return first, func(d time.Time) int {
		i := int(d.UTC().Sub(first).Hours() / 24)
		if i < 0 || i >= statsDays {
			return -1
		}
		return i
	}
}

func summary(code, url string, created time.Time, total int64, last30 []int64) linkSummary {
	return linkSummary{Code: code, URL: url, CreatedAt: created.UTC().Format(time.RFC3339), ClicksTotal: total, Last30Days: last30}
}

// linkSummaryFor loads one link's totals; used after create and patch.
func (s *server) linkSummaryFor(r *http.Request, id int64, code, url string, created time.Time) (linkSummary, error) {
	ctx := r.Context()
	total, err := s.q.LinkTotal(ctx, id)
	if err != nil {
		return linkSummary{}, err
	}
	first, slot := s.days()
	rows, err := s.q.LinkDaily(ctx, gen.LinkDailyParams{LinkID: id, Day: first})
	if err != nil {
		return linkSummary{}, err
	}
	last30 := make([]int64, statsDays)
	for _, d := range rows {
		if i := slot(d.Day); i >= 0 {
			last30[i] = d.Clicks
		}
	}
	return summary(code, url, created, total, last30), nil
}

type createLinkBody struct {
	URL   *string `json:"url"`
	Alias *string `json:"alias"`
}

func (s *server) createLink(w http.ResponseWriter, r *http.Request) {
	user, ctx := userFrom(r), r.Context()

	window := s.Now().Truncate(createWindow)
	n, err := s.q.BumpRate(ctx, gen.BumpRateParams{Key: "create:" + strconv.FormatInt(user.ID, 10), WindowStart: window})
	if err != nil {
		s.fail(w, err)
		return
	}
	if n == 1 { // first request of a new window: drop old windows so the table stays small
		_ = s.q.DeleteOldRates(ctx, s.Now().Add(-time.Hour))
	}
	if n > createPerMin {
		writeRateLimited(w, int(window.Add(createWindow).Sub(s.Now()).Seconds()))
		return
	}

	var body createLinkBody
	if readJSON(w, r, &body) != nil || body.URL == nil {
		writeProblem(w, http.StatusUnprocessableEntity, "invalid_body", "")
		return
	}
	if code := checkTargetURL(*body.URL, s.AppOrigin); code != "" {
		writeProblem(w, http.StatusUnprocessableEntity, code, "url")
		return
	}

	var row gen.CreateLinkRow
	if body.Alias != nil {
		alias, problemCode := checkAlias(*body.Alias)
		if problemCode != "" {
			writeProblem(w, http.StatusUnprocessableEntity, problemCode, "alias")
			return
		}
		row, err = s.q.CreateLink(ctx, gen.CreateLinkParams{UserID: user.ID, Code: alias, Url: *body.URL})
		if isUniqueViolation(err) {
			writeProblem(w, http.StatusConflict, "alias_taken", "alias")
			return
		}
	} else {
		for range codeAttempts { // 36^7 codes: a collision is rare, retry a few times
			var code string
			if code, err = randomCode(); err != nil {
				break
			}
			row, err = s.q.CreateLink(ctx, gen.CreateLinkParams{UserID: user.ID, Code: code, Url: *body.URL})
			if !isUniqueViolation(err) {
				break
			}
		}
	}
	if err != nil {
		s.fail(w, err)
		return
	}
	writeJSON(w, http.StatusCreated, summary(row.Code, row.Url, row.CreatedAt, 0, make([]int64, statsDays)))
}

func (s *server) listLinks(w http.ResponseWriter, r *http.Request) {
	user, ctx := userFrom(r), r.Context()
	links, err := s.q.ListLinks(ctx, user.ID)
	if err != nil {
		s.fail(w, err)
		return
	}
	first, slot := s.days()
	daily, err := s.q.DailyClicksForUser(ctx, gen.DailyClicksForUserParams{UserID: user.ID, Day: first})
	if err != nil {
		s.fail(w, err)
		return
	}
	byLink := map[int64][]int64{}
	for _, d := range daily {
		if byLink[d.LinkID] == nil {
			byLink[d.LinkID] = make([]int64, statsDays)
		}
		if i := slot(d.Day); i >= 0 {
			byLink[d.LinkID][i] = d.Clicks
		}
	}
	items := make([]linkSummary, 0, len(links))
	for _, l := range links {
		last30 := byLink[l.ID]
		if last30 == nil {
			last30 = make([]int64, statsDays)
		}
		items = append(items, summary(l.Code, l.Url, l.CreatedAt, l.ClicksTotal, last30))
	}
	writeJSON(w, http.StatusOK, map[string]any{"items": items})
}

func pathCode(r *http.Request) string { return strings.ToLower(chi.URLParam(r, "code")) }

func (s *server) patchLink(w http.ResponseWriter, r *http.Request) {
	var body struct {
		URL *string `json:"url"`
	}
	if readJSON(w, r, &body) != nil || body.URL == nil {
		writeProblem(w, http.StatusUnprocessableEntity, "invalid_body", "")
		return
	}
	if code := checkTargetURL(*body.URL, s.AppOrigin); code != "" {
		writeProblem(w, http.StatusUnprocessableEntity, code, "url")
		return
	}
	row, err := s.q.UpdateOwnLinkURL(r.Context(), gen.UpdateOwnLinkURLParams{Code: pathCode(r), UserID: userFrom(r).ID, Url: *body.URL})
	if errors.Is(err, pgx.ErrNoRows) {
		writeProblem(w, http.StatusNotFound, "not_found", "")
		return
	}
	if err != nil {
		s.fail(w, err)
		return
	}
	out, err := s.linkSummaryFor(r, row.ID, row.Code, row.Url, row.CreatedAt)
	if err != nil {
		s.fail(w, err)
		return
	}
	writeJSON(w, http.StatusOK, out)
}

func (s *server) deleteLink(w http.ResponseWriter, r *http.Request) {
	n, err := s.q.DeleteOwnLink(r.Context(), gen.DeleteOwnLinkParams{Code: pathCode(r), UserID: userFrom(r).ID})
	if err != nil {
		s.fail(w, err)
		return
	}
	if n == 0 {
		writeProblem(w, http.StatusNotFound, "not_found", "")
		return
	}
	w.WriteHeader(http.StatusNoContent)
}

type dayClicks struct {
	Date   string `json:"date"`
	Clicks int64  `json:"clicks"`
}

type referrerClicks struct {
	Host   *string `json:"host"` // null = unknown referrer
	Clicks int64   `json:"clicks"`
}

func (s *server) linkStats(w http.ResponseWriter, r *http.Request) {
	ctx := r.Context()
	link, err := s.q.GetOwnLink(ctx, gen.GetOwnLinkParams{Code: pathCode(r), UserID: userFrom(r).ID})
	if errors.Is(err, pgx.ErrNoRows) {
		writeProblem(w, http.StatusNotFound, "not_found", "")
		return
	}
	if err != nil {
		s.fail(w, err)
		return
	}
	total, err := s.q.LinkTotal(ctx, link.ID)
	if err != nil {
		s.fail(w, err)
		return
	}
	first, slot := s.days()
	rows, err := s.q.LinkDaily(ctx, gen.LinkDailyParams{LinkID: link.ID, Day: first})
	if err != nil {
		s.fail(w, err)
		return
	}
	refs, err := s.q.LinkReferrers(ctx, gen.LinkReferrersParams{LinkID: link.ID, Day: first})
	if err != nil {
		s.fail(w, err)
		return
	}
	daily := make([]dayClicks, statsDays)
	for i := range daily {
		daily[i] = dayClicks{Date: first.AddDate(0, 0, i).Format(dateLayout)}
	}
	for _, d := range rows {
		if i := slot(d.Day); i >= 0 {
			daily[i].Clicks = d.Clicks
		}
	}
	referrers := make([]referrerClicks, 0, len(refs))
	for _, ref := range refs {
		rc := referrerClicks{Clicks: ref.Clicks}
		if ref.ReferrerHost != "" {
			host := ref.ReferrerHost
			rc.Host = &host
		}
		referrers = append(referrers, rc)
	}
	writeJSON(w, http.StatusOK, map[string]any{"code": link.Code, "clicksTotal": total, "daily": daily, "referrers": referrers})
}

// redirect counts the click, then sends the visitor on. Counting happens before the response so the
// owner's stats are current, but a counting failure never blocks the redirect.
func (s *server) redirect(w http.ResponseWriter, r *http.Request) {
	ctx := r.Context()
	link, err := s.q.GetLinkTarget(ctx, pathCode(r))
	if errors.Is(err, pgx.ErrNoRows) {
		writeNotFoundPage(w)
		return
	}
	if err != nil {
		s.fail(w, err)
		return
	}
	day := s.Now().UTC().Truncate(24 * time.Hour)
	if err := s.q.CountClick(ctx, gen.CountClickParams{LinkID: link.ID, Day: day, ReferrerHost: referrerHost(r.Referer())}); err != nil {
		s.Log.Error("count_click_failed", "error", err.Error())
	}
	w.Header().Set("Cache-Control", "no-store")
	http.Redirect(w, r, link.Url, http.StatusFound)
}
