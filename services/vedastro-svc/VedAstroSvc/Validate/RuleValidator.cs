using System.Diagnostics;
using VedAstro.Library;
using VedAstroSvc.Contracts;
using VedAstroSvc.Rules;

namespace VedAstroSvc.Validate;

public sealed class RuleValidator
{
    public const int MaxRowsCap = 15807;
    public const int PromoteMinFired = 200;

    private readonly RuleCatalog _catalog;
    private readonly PredicateRegistry _predicates;
    private readonly DatasetStore _store;
    private readonly Evidence _evidence;
    private readonly PromotionLog _promotions;

    public const string PromotionScope = "natal_confidence_reporting_only; never changes rule_status, the XML, or muhurta.find eligibility (quarantined rules stay excluded)";

    public RuleValidator(RuleCatalog catalog, PredicateRegistry predicates, DatasetStore store, Evidence evidence, PromotionLog promotions)
    {
        _catalog = catalog;
        _predicates = predicates;
        _store = store;
        _evidence = evidence;
        _promotions = promotions;
    }

    public PromotionLog Promotions => _promotions;

    public string? Validate(RuleValidateRequest req, out RuleEntry? rule)
    {
        rule = null;
        if (string.IsNullOrWhiteSpace(req.RuleId)) { return "rule_id is required"; }
        rule = _catalog.Find(req.RuleId.Trim());
        if (rule is null) { return $"unknown rule '{req.RuleId}'"; }
        if (rule.Set != RuleSet.Horoscope) { return $"'{req.RuleId}' is a muhurta event rule; rule.validate evaluates natal (horoscope) rules on birth charts"; }
        if (!rule.HasPredicate) { return $"'{req.RuleId}' has no C# predicate in any public VedAstro source (predicate_missing) — cannot be evaluated"; }
        if (string.IsNullOrWhiteSpace(req.Dataset) || !_store.HasDataset(req.Dataset)) { return $"dataset must be one of: {string.Join(", ", DatasetStore.OutcomeColumns.Keys)}"; }
        if (string.IsNullOrWhiteSpace(req.OutcomeColumn) || !_store.HasOutcome(req.Dataset, req.OutcomeColumn)) { return $"outcome_column for '{req.Dataset}' must be one of: {string.Join(", ", DatasetStore.OutcomeColumns[req.Dataset])}"; }
        if (req.MaxRows < 1 || req.MaxRows > MaxRowsCap) { return $"max_rows must be in [1,{MaxRowsCap}]"; }
        if (req.Offset < 0) { return "offset must be >= 0"; }
        return null;
    }

    public RuleValidateResponse Run(RuleValidateRequest req)
    {
        var err = Validate(req, out var ruleEntry);
        if (err is not null) { throw new ArgumentException(err); }
        var rule = ruleEntry!;
        var pred = _predicates.Horoscope(rule.Id)!;
        var sw = Stopwatch.StartNew();
        var rows = _store.Rows(req.Dataset, req.OutcomeColumn, req.Offset, req.MaxRows);

        int n = 0, fired = 0, hits = 0, outcomeTrue = 0, errors = 0;
        try
        {
            foreach (var (person, outcome) in rows)
            {
                bool occurring;
                try
                {
                    var geo = new GeoLocation(person.LocationName, person.Lon, person.Lat);
                    var time = new Time(person.StdTime, geo);
                    occurring = pred(time).Occuring;
                }
                catch (Exception)
                {
                    errors++;
                    continue;
                }
                n++;
                var truthy = outcome > 0;
                if (truthy) { outcomeTrue++; }
                if (occurring)
                {
                    fired++;
                    if (truthy) { hits++; }
                }
            }
        }
        finally
        {
            CacheManager.ResetAll();
        }

        var (verdict, reason, hitRate, baseRate, lo, hi) = Decide(n, fired, hits, outcomeTrue);
        var datasetSha = req.Dataset == "person" ? _store.PersonsSha256 : _store.MarriageSha256;

        // Blueprint §6: every verdict is a row in the hash-chained promotion log. No row → no verdict (fail closed):
        // PromotionLogException propagates to the HTTP layer as 503 promotion_log_unavailable.
        var entry = _promotions.Append(new PromotionEntry(
            Ts: DateTimeOffset.UtcNow.ToString("yyyy-MM-dd'T'HH:mm:ss.fffffff'Z'", System.Globalization.CultureInfo.InvariantCulture),
            RuleId: rule.Id,
            Dataset: req.Dataset,
            DatasetSha256: datasetSha,
            OutcomeColumn: req.OutcomeColumn,
            N: n,
            Fired: fired,
            Hits: hits,
            HitRate: Math.Round(hitRate, 6),
            BaseRate: Math.Round(baseRate, 6),
            Ci95: new[] { Math.Round(lo, 6), Math.Round(hi, 6) },
            Verdict: verdict,
            PrevHash: PromotionLog.GenesisHash));

        // Promotion state is read back from the file, never from the value we just computed.
        var promotionStatus = _promotions.StatusOf(rule.Id);
        var validationStatus = ValidationStatus(promotionStatus, rule.Status);

        return new RuleValidateResponse(
            RuleId: rule.Id,
            RuleStatus: rule.Status.ToString().ToLowerInvariant(),
            Dataset: req.Dataset,
            OutcomeColumn: req.OutcomeColumn,
            N: n,
            Fired: fired,
            Hits: hits,
            HitRate: Math.Round(hitRate, 6),
            BaseRate: Math.Round(baseRate, 6),
            BaseRateFullDataset: Math.Round(_store.BaseRateFull(req.Dataset, req.OutcomeColumn), 6),
            Ci95: new Ci95(Math.Round(lo, 6), Math.Round(hi, 6)),
            Verdict: verdict,
            VerdictReason: reason,
            ValidationStatus: validationStatus,
            PromotionStatus: promotionStatus,
            PromotionScope: PromotionScope,
            PromotionEntryHash: entry.EntryHash!,
            RowErrors: errors,
            ElapsedMs: sw.ElapsedMilliseconds,
            Evidence: _evidence with { DatasetSha256 = datasetSha });
    }

    /// <summary>
    /// The promotion rule (blueprint §4 VedAstro): PROMOTE only if <c>fired ≥ 200</c> and the Wilson 95% lower bound on
    /// hits/fired exceeds the base rate over the same rows; otherwise KEEP_UNPROVED. Pure function so the gate is testable
    /// without a 200-row ephemeris run.
    /// </summary>
    public static (string Verdict, string Reason, double HitRate, double BaseRate, double Lo, double Hi) Decide(int n, int fired, int hits, int outcomeTrue)
    {
        if (n < 0 || fired < 0 || hits < 0 || outcomeTrue < 0 || fired > n || hits > fired || outcomeTrue > n)
        {
            throw new ArgumentOutOfRangeException(nameof(n), $"inconsistent counts n={n} fired={fired} hits={hits} outcome_true={outcomeTrue}");
        }
        var hitRate = fired == 0 ? 0 : hits / (double)fired;
        var baseRate = n == 0 ? 0 : outcomeTrue / (double)n;
        var (lo, hi) = Wilson95(hits, fired);
        if (fired < PromoteMinFired)
        {
            return ("KEEP_UNPROVED", $"rule fired on {fired} rows; PROMOTE requires >= {PromoteMinFired} firing rows and ci95.lo > base_rate", hitRate, baseRate, lo, hi);
        }
        if (lo > baseRate)
        {
            return ("PROMOTE", $"ci95.lo {lo:F4} > base_rate {baseRate:F4} with {fired} firing rows", hitRate, baseRate, lo, hi);
        }
        return ("KEEP_UNPROVED", $"ci95.lo {lo:F4} <= base_rate {baseRate:F4}", hitRate, baseRate, lo, hi);
    }

    /// <summary>
    /// <c>validation_status</c> as the blueprint (§4 VedAstro, "every prediction carries validation_status") wants it:
    /// PROMOTED, PROMOTED_STILL_QUARANTINED (log says PROMOTE but the rule still lives only in the not-proved XML, so it
    /// stays out of muhurta.find and keeps rule_status quarantined until a human moves it), or NOT_PROMOTED.
    /// </summary>
    public static string ValidationStatus(string promotionStatus, RuleStatus ruleStatus) => promotionStatus switch
    {
        PromotionLog.PromotedStatus => ruleStatus == RuleStatus.Quarantined ? "PROMOTED_STILL_QUARANTINED" : "PROMOTED",
        _ => "NOT_PROMOTED",
    };

    /// <summary>Wilson score interval, z = 1.959964 (95%). Returns (0,0) when n = 0.</summary>
    public static (double Lo, double Hi) Wilson95(int successes, int n)
    {
        if (n <= 0) { return (0, 0); }
        const double z = 1.959963984540054;
        var p = successes / (double)n;
        var z2 = z * z;
        var denom = 1 + z2 / n;
        var center = (p + z2 / (2 * n)) / denom;
        var half = z * Math.Sqrt(p * (1 - p) / n + z2 / (4.0 * n * n)) / denom;
        return (Math.Max(0, center - half), Math.Min(1, center + half));
    }
}
