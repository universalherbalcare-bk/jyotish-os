using System;
using System.Collections.Generic;
using System.Linq;
using SwissEphNet;

namespace VedAstro.Library
{
    /// <summary>
    /// Thrown by a rule predicate whose upstream implementation is not available in any public
    /// VedAstro source (lost in upstream commit 319a610f, 2023-09-28) and was NOT reconstructed here.
    /// vedastro-svc catches this and reports the rule as "unevaluable" instead of guessing.
    /// </summary>
    public sealed class RulePredicateUnavailableException : NotSupportedException
    {
        public RulePredicateUnavailableException(string member) : base($"Calculate.{member}: upstream implementation not present in public source (removed 2023-09-28, never re-added); not reconstructed — fails closed.") { }
    }

    /// <summary>
    /// jyotish-os adapter layer: the 2026 callers (Core.cs / Muhurtha.cs / Ashtakavarga.cs / Vargas.cs)
    /// use names and argument orders that differ from the 2023-09-28 implementations restored in
    /// Calculate.Restored-2023-09-28.cs. Everything here is either (a) a pure rename/argument-order
    /// forwarder, (b) a small well-defined computation (documented inline), or (c) a fail-closed stub.
    /// Positions come from SwissEphNet 2.8 in SIDEREAL mode with <see cref="Ayanamsa"/> (default LAHIRI),
    /// replacing the 2023 "year-of-coincidence x 50.33 arcsec" approximation.
    /// </summary>
    public static partial class Calculate
    {
        // ---------------------------------------------------------------- canonical config (CONTRACT.md §1)

        /// <summary>Swiss Ephemeris sidereal mode id. CONTRACT.md: LAHIRI (=1). Settable for tests only.</summary>
        public static int Ayanamsa { get; set; } = (int)Library.Ayanamsa.LAHIRI;

        /// <summary>CONTRACT.md: TRUE node. false => SE_TRUE_NODE for Rahu/Ketu.</summary>
        public static bool UseMeanRahuKetu { get; set; } = false;

        /// <summary>
        /// Days per solar year used by VimshottariDasa.cs. Upstream value not in public source;
        /// ASSUMPTION: 365.25 (Julian year) unless VimshottariDasa.cs overrides for RAMAN/KP.
        /// </summary>
        public static double SolarYearTimeSpan { get; set; } = 365.25;

        // ---------------------------------------------------------------- time <-> julian

        /// <summary>Julian Day (UT) of the instant. Gregorian calendar, computed from the UTC form of the time.</summary>
        public static double TimeToJulianUniversalTime(Time time)
        {
            var utc = time.GetStdDateTimeOffset().ToUniversalTime();
            var eph = new SwissEph();
            return eph.swe_julday(utc.Year, utc.Month, utc.Day, utc.TimeOfDay.TotalHours, SwissEph.SE_GREG_CAL);
        }

        /// <summary>Julian Day (ET/TT) = UT + deltaT, same as the 2023 TimeToEphemerisTime.</summary>
        public static double TimeToJulianEphemerisTime(Time time) => TimeToEphemerisTime(time);

        /// <summary>Ayanamsa in degrees at the instant, from Swiss Ephemeris for the configured sidereal mode.</summary>
        public static Angle AyanamsaDegree(Time time)
        {
            var eph = new SwissEph();
            eph.swe_set_sid_mode(Ayanamsa, 0, 0);
            var jdUt = TimeToJulianUniversalTime(time);
            return Angle.FromDegrees(eph.swe_get_ayanamsa_ut(jdUt));
        }

        // ---------------------------------------------------------------- positions (sidereal, Swiss 2.8)

        /// <summary>
        /// Sidereal (nirayana) longitude of a planet. Swiss Ephemeris, SEFLG_SIDEREAL with the configured
        /// ayanamsa; Rahu = true node unless <see cref="UseMeanRahuKetu"/>; Ketu = Rahu + 180.
        /// Moshier fallback is used when no .se1 files are present (none are shipped).
        /// </summary>
        public static Angle PlanetNirayanaLongitude(Time time, PlanetName planetName)
        {
            return CacheManager.GetCache(new CacheKey(nameof(PlanetNirayanaLongitude), time, planetName, Ayanamsa, UseMeanRahuKetu), _get);

            Angle _get()
            {
                var eph = new SwissEph();
                eph.swe_set_sid_mode(Ayanamsa, 0, 0);
                const int iflag = SwissEph.SEFLG_SWIEPH | SwissEph.SEFLG_SIDEREAL;
                var results = new double[6];
                var err = "";
                var jdEt = TimeToEphemerisTime(time);
                var swissPlanet = Tools.VedAstroToSwissEph(planetName);
                var ret = eph.swe_calc(jdEt, swissPlanet, iflag, results, ref err);
                if (ret < 0) { throw new InvalidOperationException($"swe_calc failed for {planetName}: {err}"); }
                var lon = Angle.FromDegrees(results[0]);
                if (planetName == PlanetName.Ketu) { lon = (lon + Angle.Degrees180).Normalize360(); }
                return lon;
            }
        }

        public static Angle PlanetNirayanaLongitude(PlanetName planetName, Time time) => PlanetNirayanaLongitude(time, planetName);
        public static Angle PlanetSayanaLongitude(PlanetName planetName, Time time) => PlanetSayanaLongitude(time, planetName);

        // ---------------------------------------------------------------- pure renames / argument-order forwarders

        /// <summary>2026 name for the 2023 PlanetSignName (rasi / D1 sign of a planet).</summary>
        public static ZodiacSign PlanetRasiD1Sign(PlanetName planetName, Time time) => PlanetSignName(planetName, time);

        public static List<PlanetName> PlanetsAspectingPlanet(PlanetName receivingAspect, Time time) => PlanetsAspectingPlanet(time, receivingAspect);

        public static bool IsPlanetSameHouseWithHouseLord(int houseNumber, PlanetName planet, Time birthTime) => IsPlanetSameHouseWithHouseLord(birthTime, houseNumber, planet);

        /// <summary>2026 name for 2023 IsPlanetBeneficInShadbala: shadbala pinda meets the classical minimum (B.V. Raman rupas).</summary>
        public static bool IsPlanetStrongInShadbala(PlanetName planet, Time time) => IsPlanetBeneficInShadbala(planet, time);

        /// <summary>Signs at the middle longitude of every house (Core.cs AllHouseLongitudes), keyed by house.</summary>
        public static Dictionary<HouseName, ZodiacSign> AllHouseZodiacSigns(Time time)
        {
            var dict = new Dictionary<HouseName, ZodiacSign>();
            foreach (var house in AllHouseLongitudes(time))
            {
                dict[house.GetHouseName()] = ZodiacSignAtLongitude(house.GetMiddleLongitude());
            }
            return dict;
        }

        /// <summary>Signs ruled by a planet (inverse of LordOfZodiacSign). Rahu/Ketu rule none.</summary>
        public static List<ZodiacName> ZodiacSignsOwnedByPlanet(PlanetName planet)
        {
            var owned = new List<ZodiacName>();
            foreach (var sign in ZodiacNameExtensions.AllZodiacSigns)
            {
                if (LordOfZodiacSign(sign) == planet) { owned.Add(sign); }
            }
            return owned;
        }

        /// <summary>
        /// Degrees inside the divisional sign for a D-n chart: each sign is split into n equal parts of
        /// 30/n degrees; the position inside the part is scaled back to a 0..30 range.
        /// </summary>
        public static Angle DivisionalLongitude(Angle degreesInSign, int divisionNumber)
        {
            if (divisionNumber <= 0) { throw new ArgumentOutOfRangeException(nameof(divisionNumber)); }
            var part = 30.0 / divisionNumber;
            var inPart = degreesInSign.TotalDegrees % part;
            if (inPart < 0) { inPart += part; }
            return Angle.FromDegrees(inPart * divisionNumber);
        }

        public static Angle DivisionalLongitude(double degreesInSign, int divisionNumber) => DivisionalLongitude(Angle.FromDegrees(degreesInSign), divisionNumber);

        /// <summary>Local mean time (date + longitude) to standard time: LMT offset = 4 min/degree, then re-expressed at the standard offset.</summary>
        public static DateTimeOffset LmtToStd(LocalMeanTime lmt, TimeSpan stdOffset)
        {
            var lmtDto = new DateTimeOffset(DateTime.SpecifyKind(lmt.Date, DateTimeKind.Unspecified), LongitudeToLMTOffset(lmt.Longitude));
            return lmtDto.ToOffset(stdOffset);
        }

        /// <summary>Navamsa (D9) sign of a house, computed from the house's middle longitude via Vargas.NavamshaTable.</summary>
        public static ZodiacSign HouseNavamshaD9Sign(HouseName house, Time time)
        {
            var rasi = ZodiacSignAtLongitude(HouseLongitude(house, time).GetMiddleLongitude());
            return Vargas.VargasCoreCalculator(rasi, Vargas.NavamshaTable, 9);
        }

        /// <summary>Navamsa (D9) sign of a planet via Vargas.NavamshaTable.</summary>
        public static ZodiacSign PlanetNavamshaD9Sign(PlanetName planet, Time time)
        {
            var rasi = PlanetSignName(planet, time);
            return Vargas.VargasCoreCalculator(rasi, Vargas.NavamshaTable, 9);
        }

        /// <summary>Bhinnashtakavarga for the 7 planets, wrapped in the 2026 data type.</summary>
        public static Bhinnashtakavarga BhinnashtakavargaChart(Time birthTime)
        {
            var chart = new Bhinnashtakavarga();
            foreach (var kv in AllBhinnashtakavargaChart(birthTime)) { chart[kv.Key] = kv.Value; }
            return chart;
        }

        /// <summary>Local-mean-time offset for a longitude: 4 minutes per degree east of Greenwich.</summary>
        public static TimeSpan LongitudeToLMTOffset(double longitudeDeg)
        {
            var minutes = longitudeDeg * 4.0;
            return TimeSpan.FromMinutes(Math.Round(minutes, 0));
        }

        // ---------------------------------------------------------------- fail-closed stubs (no network / no lost logic)

        /// <summary>Upstream: Google/Azure Maps geocoding. Forbidden in jyotish-os (CONTRACT.md).</summary>
        public static GeoLocation AddressToGeoLocation(string address) =>
            throw new NotSupportedException("AddressToGeoLocation: runtime geocoding is disabled; pass lat/lon.");

        /// <summary>Upstream: timezone API. Forbidden in jyotish-os; callers pass tz_offset_hours.</summary>
        public static System.Threading.Tasks.Task<string> GeoLocationToTimezone(GeoLocation geoLocation, DateTimeOffset timeAtLocation) =>
            throw new NotSupportedException("GeoLocationToTimezone: runtime timezone lookup is disabled; pass tz offset.");

        /// <summary>Pancha Pakshi main activity. Upstream implementation lost; not reconstructed (see exception).</summary>
        public static BirdActivity MainActivity(Time birthTime, Time time) => throw new RulePredicateUnavailableException(nameof(MainActivity));

        /// <summary>Pancha Pakshi yama of the instant. Upstream implementation lost; not reconstructed (see exception).</summary>
        public static BirthYama BirthYama(Time time) => throw new RulePredicateUnavailableException(nameof(BirthYama));
    }
}
