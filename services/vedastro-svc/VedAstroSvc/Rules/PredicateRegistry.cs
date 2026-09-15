using System.Reflection;
using VedAstro.Library;

namespace VedAstroSvc.Rules;

/// <summary>
/// One-time reflection over VedAstro's predicate classes:
///   EventCalculatorMethods  (Muhurtha.cs, [EventCalculator(EventName.X)])       -> (Time, Person) => CalculatorResult
///   CalculateHoroscope      (restored 2023 file, [HoroscopeCalculator(HoroscopeName.X)]) -> (Time) => CalculatorResult
/// Upstream's EventManager.GetEventCalculatorMethod re-scans all methods on every call; this caches
/// the delegates by XML name so a muhurta sweep of 2,000 slices x 40 rules costs no reflection.
/// </summary>
public sealed class PredicateRegistry
{
    private readonly Dictionary<string, EventCalculatorDelegate> _event = new(StringComparer.Ordinal);
    private readonly Dictionary<string, HoroscopeCalculatorDelegate> _horoscope = new(StringComparer.Ordinal);

    public PredicateRegistry()
    {
        foreach (var m in typeof(EventCalculatorMethods).GetMethods(BindingFlags.Public | BindingFlags.Static))
        {
            var attr = m.GetCustomAttribute<EventCalculatorAttribute>();
            if (attr is null) { continue; }
            var name = attr.EventName.ToString();
            if (_event.ContainsKey(name)) { continue; }
            if (Delegate.CreateDelegate(typeof(EventCalculatorDelegate), m, throwOnBindFailure: false) is EventCalculatorDelegate d)
            {
                _event[name] = d;
            }
        }
        foreach (var m in typeof(CalculateHoroscope).GetMethods(BindingFlags.Public | BindingFlags.Static))
        {
            var attr = m.GetCustomAttribute<HoroscopeCalculatorAttribute>();
            if (attr is null) { continue; }
            var name = attr.HoroscopeName.ToString();
            if (_horoscope.ContainsKey(name)) { continue; }
            if (Delegate.CreateDelegate(typeof(HoroscopeCalculatorDelegate), m, throwOnBindFailure: false) is HoroscopeCalculatorDelegate d)
            {
                _horoscope[name] = d;
            }
        }
    }

    public int EventPredicateCount => _event.Count;
    public int HoroscopePredicateCount => _horoscope.Count;
    public bool HasEventPredicate(string name) => _event.ContainsKey(name);
    public bool HasHoroscopePredicate(string name) => _horoscope.ContainsKey(name);
    public EventCalculatorDelegate? Event(string name) => _event.TryGetValue(name, out var d) ? d : null;
    public HoroscopeCalculatorDelegate? Horoscope(string name) => _horoscope.TryGetValue(name, out var d) ? d : null;
}
