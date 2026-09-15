using System.Xml.Linq;

namespace VedAstroSvc.Rules;

public enum RuleSet { Event, Horoscope }
public enum RuleStatus { Proved, Quarantined }

/// <summary>One rule as declared in VedAstro's XML data. <c>Id</c> is the XML &lt;Name&gt; (it is what the C# predicate attribute keys on).</summary>
public sealed record RuleEntry(
    string Id,
    string Name,
    RuleSet Set,
    RuleStatus Status,
    string Nature,
    string Description,
    IReadOnlyList<string> Tags,
    bool HasPredicate);

/// <summary>
/// Loads the four VedAstro rule files. A rule is PROVED when it appears in EventDataList.xml /
/// HoroscopeDataList.xml. A rule that appears ONLY in the corresponding "-not-proved.xml" file is
/// QUARANTINED. (EventDataList-not-proved.xml is a superset of the proved file: 1018 shared + 10 extra.)
/// Counts are taken by parsing, never by grepping: HoroscopeDataList.xml contains one &lt;Event&gt;
/// inside an XML comment, so its parsed count is 490, not 491.
/// </summary>
public sealed class RuleCatalog
{
    private readonly Dictionary<string, RuleEntry> _byId;

    public IReadOnlyList<RuleEntry> All { get; }
    public string XmlDirectory { get; }
    public IReadOnlyDictionary<string, string> FileSha256 { get; }

    public RuleCatalog(string xmlDirectory, PredicateRegistry predicates)
    {
        XmlDirectory = xmlDirectory;
        var files = new Dictionary<string, string>();
        var list = new List<RuleEntry>();

        var eventProved = Parse(Path.Combine(xmlDirectory, "EventDataList.xml"), files);
        var eventNotProved = Parse(Path.Combine(xmlDirectory, "EventDataList-not-proved.xml"), files);
        var horoProved = Parse(Path.Combine(xmlDirectory, "HoroscopeDataList.xml"), files);
        var horoNotProved = Parse(Path.Combine(xmlDirectory, "HoroscopeDataList-not-proved.xml"), files);

        AddSet(list, eventProved, eventNotProved, RuleSet.Event, predicates);
        AddSet(list, horoProved, horoNotProved, RuleSet.Horoscope, predicates);

        All = list;
        _byId = new Dictionary<string, RuleEntry>(StringComparer.Ordinal);
        foreach (var r in list) { _byId[r.Id] = r; }
        FileSha256 = files;
    }

    public RuleEntry? Find(string id) => _byId.TryGetValue(id, out var r) ? r : null;

    public IEnumerable<RuleEntry> Query(RuleSet? set, RuleStatus? status) =>
        All.Where(r => (set is null || r.Set == set) && (status is null || r.Status == status));

    public int CountProved => All.Count(r => r.Status == RuleStatus.Proved);
    public int CountQuarantined => All.Count(r => r.Status == RuleStatus.Quarantined);
    public int Count(RuleSet set, RuleStatus status) => All.Count(r => r.Set == set && r.Status == status);
    public int CountWithPredicate(RuleSet set, RuleStatus status) => All.Count(r => r.Set == set && r.Status == status && r.HasPredicate);

    private static void AddSet(List<RuleEntry> sink, List<RawRule> proved, List<RawRule> notProved, RuleSet set, PredicateRegistry predicates)
    {
        var provedNames = new HashSet<string>(proved.Select(r => r.Name), StringComparer.Ordinal);
        var seen = new HashSet<string>(StringComparer.Ordinal);
        foreach (var r in proved)
        {
            if (!seen.Add(r.Name)) { continue; }
            sink.Add(ToEntry(r, set, RuleStatus.Proved, predicates));
        }
        foreach (var r in notProved)
        {
            if (provedNames.Contains(r.Name) || !seen.Add(r.Name)) { continue; }
            sink.Add(ToEntry(r, set, RuleStatus.Quarantined, predicates));
        }
    }

    private static RuleEntry ToEntry(RawRule r, RuleSet set, RuleStatus status, PredicateRegistry predicates)
    {
        var has = set == RuleSet.Event ? predicates.HasEventPredicate(r.Name) : predicates.HasHoroscopePredicate(r.Name);
        return new RuleEntry(r.Name, r.Name, set, status, r.Nature, r.Description, r.Tags, has);
    }

    private sealed record RawRule(string Name, string Nature, string Description, IReadOnlyList<string> Tags);

    private static List<RawRule> Parse(string path, Dictionary<string, string> shaSink)
    {
        if (!File.Exists(path)) { throw new FileNotFoundException("VedAstro rule file missing", path); }
        var bytes = File.ReadAllBytes(path);
        shaSink[Path.GetFileName(path)] = Convert.ToHexStringLower(System.Security.Cryptography.SHA256.HashData(bytes));
        var doc = XDocument.Load(new MemoryStream(bytes));
        var root = doc.Root ?? throw new InvalidDataException($"{path}: no root element");
        var rules = new List<RawRule>();
        foreach (var ev in root.Elements("Event"))
        {
            var name = (ev.Element("Name")?.Value ?? "").Trim();
            if (name.Length == 0) { throw new InvalidDataException($"{path}: <Event> without <Name>"); }
            var nature = (ev.Element("Nature")?.Value ?? "").Trim();
            var desc = Normalize(ev.Element("Description")?.Value ?? "");
            var cond = Normalize(ev.Element("ConditionDescription")?.Value ?? "");
            if (cond.Length > 0) { desc = desc.Length > 0 ? $"IF {cond} THEN {desc}" : cond; }
            var tags = ev.Elements("Tag")
                .SelectMany(t => t.Value.Split(',', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries))
                .Where(t => t.Length > 0)
                .Distinct(StringComparer.Ordinal)
                .ToList();
            rules.Add(new RawRule(name, nature, desc, tags));
        }
        return rules;
    }

    private static string Normalize(string s) =>
        string.Join(' ', s.Split((char[]?)null, StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries));
}
