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

    public RuleValidator(RuleCatalog catalog, PredicateRegistry predicates, DatasetStore store, Evidence evidence)
    {
        _catalog = catalog;
        _predicates = predicates;
        _store = store;
        _evidence = evidence;
    }

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

        var hitRate = fired == 0 ? 0 : hits / (double)fired;
        var baseRate = n == 0 ? 0 : outcomeTrue / (double)n;
        var (lo, hi) = Wilson95(hits, fired);
        string verdict, reason;
        if (fired < PromoteMinFired)
        {
            verdict = "KEEP_UNPROVED";
            reason = $"rule fired on {fired} rows; PROMOTE requires >= {PromoteMinFired} firing rows and ci95.lo > base_rate";
        }
        else if (lo > baseRate)
        {
            verdict = "PROMOTE";
            reason = $"ci95.lo {lo:F4} > base_rate {baseRate:F4} with {fired} firing rows";
        }
        else
        {
            verdict = "KEEP_UNPROVED";
            reason = $"ci95.lo {lo:F4} <= base_rate {baseRate:F4}";
        }

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
            RowErrors: errors,
            ElapsedMs: sw.ElapsedMilliseconds,
            Evidence: _evidence with { DatasetSha256 = req.Dataset == "person" ? _store.PersonsSha256 : _store.MarriageSha256 });
    }

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
