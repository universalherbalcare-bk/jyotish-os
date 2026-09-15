using System.Globalization;
using VedAstro.Library;
using VedAstroSvc.Contracts;
using VedAstroSvc.Rules;

namespace VedAstroSvc.Muhurta;

public sealed class MuhurtaFinder
{
    public const int MaxSlices = 4000;
    public const int MinStepMinutes = 1;
    public const int MaxStepMinutes = 24 * 60;

    private readonly RuleCatalog _catalog;
    private readonly PredicateRegistry _predicates;
    private readonly Evidence _evidence;

    public MuhurtaFinder(RuleCatalog catalog, PredicateRegistry predicates, Evidence evidence)
    {
        _catalog = catalog;
        _predicates = predicates;
        _evidence = evidence;
    }

    public static bool TryParseUtc(string? text, out DateTimeOffset utc)
    {
        utc = default;
        if (string.IsNullOrWhiteSpace(text)) { return false; }
        if (!DateTimeOffset.TryParse(text, CultureInfo.InvariantCulture, DateTimeStyles.AssumeUniversal | DateTimeStyles.AdjustToUniversal, out var parsed)) { return false; }
        utc = parsed.ToUniversalTime();
        return true;
    }

    /// <summary>Validates the request; returns an error message or null.</summary>
    public string? Validate(MuhurtaFindRequest req, out DateTimeOffset from, out DateTimeOffset to, out RuleEntry? activity)
    {
        from = default; to = default; activity = null;
        if (string.IsNullOrWhiteSpace(req.Activity)) { return "activity is required (a rule Name from GET /v1/rules?set=event)"; }
        activity = _catalog.Find(req.Activity.Trim());
        if (activity is null) { return $"unknown activity rule '{req.Activity}'"; }
        if (activity.Set != RuleSet.Event) { return $"'{req.Activity}' is a horoscope rule, not a muhurta event rule"; }
        if (activity.Status == RuleStatus.Quarantined && !req.IncludeQuarantined) { return $"'{req.Activity}' is quarantined (from EventDataList-not-proved.xml); set include_quarantined:true to evaluate it (results will be tagged)"; }
        if (!TryParseUtc(req.FromUtc, out from)) { return "from_utc must be an ISO-8601 instant"; }
        if (!TryParseUtc(req.ToUtc, out to)) { return "to_utc must be an ISO-8601 instant"; }
        if (to <= from) { return "to_utc must be after from_utc"; }
        if (req.StepMinutes < MinStepMinutes || req.StepMinutes > MaxStepMinutes) { return $"step_minutes must be in [{MinStepMinutes},{MaxStepMinutes}]"; }
        if (double.IsNaN(req.Lat) || req.Lat < -90 || req.Lat > 90) { return "lat must be in [-90,90]"; }
        if (double.IsNaN(req.Lon) || req.Lon < -180 || req.Lon > 180) { return "lon must be in [-180,180]"; }
        if (double.IsNaN(req.TzOffsetHours) || req.TzOffsetHours < -14 || req.TzOffsetHours > 14) { return "tz_offset_hours must be in [-14,14]"; }
        var slices = (long)Math.Floor((to - from).TotalMinutes / req.StepMinutes) + 1;
        if (slices > MaxSlices) { return $"range/step yields {slices} slices; maximum is {MaxSlices} (widen step_minutes or narrow the range)"; }
        if (req.Birth is not null)
        {
            if (!TryParseUtc(req.Birth.Utc, out _)) { return "birth.utc must be an ISO-8601 instant"; }
            if (req.Birth.Lat < -90 || req.Birth.Lat > 90 || req.Birth.Lon < -180 || req.Birth.Lon > 180) { return "birth lat/lon out of range"; }
        }
        return null;
    }

    public MuhurtaFindResponse Find(MuhurtaFindRequest req)
    {
        var err = Validate(req, out var from, out var to, out var activityEntry);
        if (err is not null) { throw new ArgumentException(err); }
        var activity = activityEntry!;

        var tz = TimeSpan.FromHours(req.TzOffsetHours);
        var geo = new GeoLocation($"{req.Lat:F4},{req.Lon:F4}", req.Lon, req.Lat);

        // Person: most muhurta rules are universal, but Tarabala/Chandrabala-style rules read
        // person.BirthTime. Without a birth we evaluate against a placeholder born at the window
        // start and say so in the response (birth_supplied=false).
        Person person;
        if (req.Birth is not null)
        {
            TryParseUtc(req.Birth.Utc, out var birthUtc);
            var birthGeo = new GeoLocation("birth", req.Birth.Lon, req.Birth.Lat);
            person = new Person("caller", new Time(birthUtc.ToOffset(TimeSpan.FromHours(req.Birth.TzOffsetHours)), birthGeo), Gender.Male);
        }
        else
        {
            person = new Person("placeholder", new Time(from.ToOffset(tz), geo), Gender.Male);
        }

        // Sibling rules: every event rule sharing at least one tag with the activity rule.
        var tagSet = new HashSet<string>(activity.Tags, StringComparer.Ordinal);
        var candidates = _catalog.Query(RuleSet.Event, null)
            .Where(r => r.Id == activity.Id || r.Tags.Any(tagSet.Contains))
            .Where(r => r.HasPredicate)
            .Where(r => r.Status == RuleStatus.Proved || req.IncludeQuarantined)
            .OrderBy(r => r.Id, StringComparer.Ordinal)
            .ToList();

        var unevaluable = new SortedSet<string>(StringComparer.Ordinal);
        var ruleErrors = new SortedDictionary<string, string>(StringComparer.Ordinal);
        var active = candidates.ToList();

        var sliceCount = (int)Math.Floor((to - from).TotalMinutes / req.StepMinutes) + 1;
        // per slice, per rule: 0 = not occurring, else effective nature (NatureOverride from the predicate wins over XML)
        var fired = new EventNature[sliceCount][];
        var sliceStart = new DateTimeOffset[sliceCount];

        try
        {
            for (var s = 0; s < sliceCount; s++)
            {
                var t = from.AddMinutes((double)s * req.StepMinutes);
                sliceStart[s] = t;
                var time = new Time(t.ToOffset(tz), geo);
                var row = new EventNature[active.Count];
                for (var i = 0; i < active.Count; i++)
                {
                    var rule = active[i];
                    if (unevaluable.Contains(rule.Id) || ruleErrors.ContainsKey(rule.Id)) { continue; }
                    var pred = _predicates.Event(rule.Id)!;
                    try
                    {
                        var result = pred(time, person);
                        if (result.Occuring)
                        {
                            row[i] = result.NatureOverride != EventNature.Empty ? result.NatureOverride : ParseNature(rule.Nature);
                        }
                    }
                    catch (RulePredicateUnavailableException)
                    {
                        unevaluable.Add(rule.Id);
                    }
                    catch (Exception ex)
                    {
                        ruleErrors[rule.Id] = ex.GetType().Name + ": " + Truncate(ex.Message, 200);
                    }
                }
                fired[s] = row;
            }
        }
        finally
        {
            CacheManager.ResetAll();
        }

        // Build windows: maximal runs of slices with identical (passed-set, vetoed-set) signature.
        var evaluated = active.Where(r => !unevaluable.Contains(r.Id) && !ruleErrors.ContainsKey(r.Id)).ToList();
        var idx = active.Select((r, i) => (r, i)).ToDictionary(x => x.r.Id, x => x.i, StringComparer.Ordinal);

        var windows = new List<(int startSlice, int endSlice, Dictionary<string, int> good, Dictionary<string, int> bad, Dictionary<string, int> neutral, Dictionary<string, int> qgood, Dictionary<string, int> qbad, Dictionary<string, int> qneutral, int activityHits)>();
        string? prevSig = null;
        for (var s = 0; s < sliceCount; s++)
        {
            var sig = string.Join('|', evaluated.Select(r => ((int)fired[s][idx[r.Id]]).ToString(CultureInfo.InvariantCulture)));
            if (prevSig is null || sig != prevSig)
            {
                windows.Add((s, s, new(), new(), new(), new(), new(), new(), 0));
                prevSig = sig;
            }
            var w = windows[^1];
            w.endSlice = s;
            foreach (var r in evaluated)
            {
                var nature = fired[s][idx[r.Id]];
                if (nature == EventNature.Empty) { continue; }
                var q = r.Status == RuleStatus.Quarantined;
                var target = nature switch
                {
                    EventNature.Good => q ? w.qgood : w.good,
                    EventNature.Bad => q ? w.qbad : w.bad,
                    _ => q ? w.qneutral : w.neutral,
                };
                target[r.Id] = target.TryGetValue(r.Id, out var c) ? c + 1 : 1;
                if (r.Id == activity.Id) { w.activityHits++; }
            }
            windows[^1] = w;
        }

        // Score (proved rules only; quarantined rules are reported but never rank):
        //   +1 per Good-rule slice-hit, -1 per Bad-rule slice-hit, Neutral/unlabelled rules 0, averaged per slice.
        // Rank: activity rule passed > no veto > score > duration.
        var built = windows.Select(w =>
        {
            var slices = w.endSlice - w.startSlice + 1;
            var goodHits = w.good.Values.Sum();
            var badHits = w.bad.Values.Sum();
            var score = Math.Round((goodHits - badHits) / (double)slices, 4);
            var start = sliceStart[w.startSlice];
            var end = sliceStart[w.endSlice].AddMinutes(req.StepMinutes);
            if (end > to) { end = to; }
            // "activity rule passed" = the named rule fired in every slice of the window (for a Bad-natured
            // activity rule, passed = it never fired).
            var activityIsBad = ParseNature(activity.Nature) == EventNature.Bad;
            var activityPassed = activityIsBad ? w.activityHits == 0 : w.activityHits == slices;
            return new MuhurtaWindow(
                Rank: 0,
                StartUtc: start.ToString("yyyy-MM-dd'T'HH:mm:ss'Z'", CultureInfo.InvariantCulture),
                EndUtc: end.ToString("yyyy-MM-dd'T'HH:mm:ss'Z'", CultureInfo.InvariantCulture),
                StartLocal: start.ToOffset(tz).ToString("yyyy-MM-dd'T'HH:mm:sszzz", CultureInfo.InvariantCulture),
                EndLocal: end.ToOffset(tz).ToString("yyyy-MM-dd'T'HH:mm:sszzz", CultureInfo.InvariantCulture),
                DurationMinutes: (int)Math.Round((end - start).TotalMinutes),
                Slices: slices,
                Score: score,
                ActivityRulePassed: activityPassed,
                PassedRules: Trail(w.good, "Good", "proved"),
                VetoedBy: Trail(w.bad, "Bad", "proved"),
                FiredNeutral: Trail(w.neutral, "Neutral", "proved"),
                PassedRulesQuarantined: Trail(w.qgood, "Good", "quarantined"),
                VetoedByQuarantined: Trail(w.qbad, "Bad", "quarantined"),
                FiredNeutralQuarantined: Trail(w.qneutral, "Neutral", "quarantined"));
        })
        .OrderByDescending(w => w.ActivityRulePassed)
        .ThenBy(w => w.VetoedBy.Count > 0)
        .ThenByDescending(w => w.Score)
        .ThenByDescending(w => w.DurationMinutes)
        .ThenBy(w => w.StartUtc, StringComparer.Ordinal)
        .Select((w, i) => w with { Rank = i + 1 })
        .ToList();

        return new MuhurtaFindResponse(
            Activity: activity.Id,
            ActivityTags: activity.Tags,
            RulesEvaluated: evaluated.Where(r => r.Status == RuleStatus.Proved).Select(r => r.Id).ToList(),
            RulesQuarantinedEvaluated: evaluated.Where(r => r.Status == RuleStatus.Quarantined).Select(r => r.Id).ToList(),
            RulesUnevaluable: unevaluable.ToList(),
            RuleErrors: ruleErrors,
            SlicesTotal: sliceCount,
            StepMinutes: req.StepMinutes,
            BirthSupplied: req.Birth is not null,
            Complete: ruleErrors.Count == 0,
            Windows: built,
            Evidence: _evidence);
    }

    private static List<RuleTrailItem> Trail(Dictionary<string, int> d, string nature, string status) =>
        d.OrderBy(kv => kv.Key, StringComparer.Ordinal).Select(kv => new RuleTrailItem(kv.Key, nature, status, kv.Value)).ToList();

    /// <summary>XML Nature is one of Good / Bad / Neutral / "" (486 of 1018 event rules are unlabelled).</summary>
    internal static EventNature ParseNature(string nature) =>
        string.Equals(nature, "Good", StringComparison.OrdinalIgnoreCase) ? EventNature.Good
        : string.Equals(nature, "Bad", StringComparison.OrdinalIgnoreCase) ? EventNature.Bad
        : EventNature.Neutral;

    private static string Truncate(string s, int n) => s.Length <= n ? s : s[..n] + "...";
}
