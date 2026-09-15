using System;
using System.Collections.Generic;
using System.Linq;
using System.Text;
using System.Threading.Tasks;
using SwissEphNet;
using System.Globalization;

namespace VedAstro.Library
{
    /// <summary>
    /// TRIMMED Tools (jyotish-os). Every method below is copied verbatim from
    /// vendor/vedastro/Library/Logic/Tools.cs except <see cref="ParseTime"/>, which upstream
    /// routed through a geocoding API and here fails closed.
    /// </summary>
    public static class Tools
    {
        /// <summary>
        /// Upstream: resolves a location name via Google/Azure Maps. Runtime geocoding is
        /// forbidden in jyotish-os (CONTRACT.md) — callers must pass lat/lon/tz.
        /// </summary>
        public static Task<Time> ParseTime(string locationName, string hhmmStr, string dateStr, string monthStr, string yearStr, string offsetStr)
        {
            throw new NotSupportedException("Tools.ParseTime: runtime geocoding is disabled in vedastro-svc; construct Time from UTC + GeoLocation(lat,lon) instead.");
        }

        /// <summary>
        /// Custom hash generator for Strings. Returns consistent/deterministic values
        /// If null returns 0
        /// Note: MD5 (System.Security.Cryptography) not used because not supported in Blazor WASM
        /// </summary>
        public static int GetStringHashCode(string stringToHash)
        {
            if (stringToHash == null)
            {
                return 0;
            }

            unchecked
            {
                int hash1 = (5381 << 16) + 5381;
                int hash2 = hash1;

                for (int i = 0; i < stringToHash.Length; i += 2)
                {
                    hash1 = ((hash1 << 5) + hash1) ^ stringToHash[i];
                    if (i == stringToHash.Length - 1)
                        break;
                    hash2 = ((hash2 << 5) + hash2) ^ stringToHash[i + 1];
                }

                return hash1 + (hash2 * 1566083941);
            }

        }

        /// <summary>
        /// Gets random unique ID
        /// </summary>
        /// <param name="truncate">Optional parameter to truncate the generated ID</param>
        public static string GenerateId(int? truncate = null)
        {
            var id = Guid.NewGuid().ToString("N");
            return truncate.HasValue && truncate.Value < id.Length ? id.Substring(0, truncate.Value) : id;
        }

        /// <summary>
        /// Given any string will remove the white spaces
        /// </summary>
        public static string RemoveWhiteSpace(string stringWithSpace)
        {
            var removed = string.Join("", stringWithSpace.Split(default(string[]), StringSplitOptions.RemoveEmptyEntries));

            return removed;
        }

        /// <summary>
        /// Converts VedAstro planet name to Swiss Eph planet
        /// </summary>
        /// <returns></returns>
        public static int VedAstroToSwissEph(PlanetName planetName)
        {
            int planet = 0;

            //Convert PlanetName to SE_PLANET type
            if (planetName == PlanetName.Sun)
                planet = SwissEph.SE_SUN;
            else if (planetName == PlanetName.Moon)
            {
                planet = SwissEph.SE_MOON;
            }
            else if (planetName == PlanetName.Mars)
            {
                planet = SwissEph.SE_MARS;
            }
            else if (planetName == PlanetName.Mercury)
            {
                planet = SwissEph.SE_MERCURY;
            }
            else if (planetName == PlanetName.Jupiter)
            {
                planet = SwissEph.SE_JUPITER;
            }
            else if (planetName == PlanetName.Venus)
            {
                planet = SwissEph.SE_VENUS;
            }
            else if (planetName == PlanetName.Saturn)
            {
                planet = SwissEph.SE_SATURN;
            }
            else if (planetName == PlanetName.Earth)
            {
                planet = SwissEph.SE_EARTH;
            }
            else if (planetName == PlanetName.Rahu)
            {
                //set based on user preference
                planet = Calculate.UseMeanRahuKetu ? SwissEph.SE_MEAN_NODE : SwissEph.SE_TRUE_NODE;
            }
            else if (planetName == PlanetName.Ketu)
            {
                //NOTES:
                //the true node, which is the point where the Moon's orbit crosses the ecliptic plane
                //can also be SE_OSCU_APOG, but no need to add 180

                //set based on user preference, ask for rahu values then add 180 later
                planet = Calculate.UseMeanRahuKetu ? SwissEph.SE_MEAN_NODE : SwissEph.SE_TRUE_NODE;
            }

            return planet;
        }

        /// <summary>
        /// Converts any list to comma separated string
        /// Note: calls ToString();
        /// </summary>
        public static string ListToString<T>(List<T> list, string separator = ",")
        {
            var combinedNames = "";

            for (int i = 0; i < list.Count; i++)
            {
                //when last in row, don't add comma
                var isLastItem = i == (list.Count - 1);
                var ending = isLastItem ? "" : $"{separator} ";

                //combine to together based on type
                var item = list[i];

                combinedNames += item.ToString() + ending;

                //if (item is IToJson iToJson)
                //{
                //    //todo can wrap into jobject if needed
                //    combinedNames += iToJson.ToJson() + ending;
                //}
                //else
                //{
                //    combinedNames += item.ToString() + ending;
                //}

            }

            return combinedNames;
        }

        /// <summary>
        /// "JupiterSunPD3" --> "Jupiter"
        /// 0 based word position, default is 0 for first word
        /// </summary>
        public static string GetCamelCaseWord(string input, int wordPosition = 0)
        {
            if (string.IsNullOrEmpty(input))
            {
                return string.Empty;
            }
            var words = new List<string>();
            var word = new StringBuilder();
            foreach (var ch in input)
            {
                if (char.IsUpper(ch) && word.Length > 0)
                {
                    words.Add(word.ToString());
                    word.Clear();
                }
                word.Append(ch);
            }
            words.Add(word.ToString());
            return words[wordPosition];
        }

        /// <summary>
        /// takes a duration in hours and
        /// returns a string that represents the duration in
        /// a more human-readable format (hours, days, months, or years). 
        /// </summary>
        public static string TimeDurationToHumanText(double durationInHours)
        {
            const double hoursInDay = 24;
            const double daysInMonth = 30;
            const double monthsInYear = 12;
            if (durationInHours < hoursInDay)
            {
                return $"{Math.Round(durationInHours, 1)} hours";
            }
            double durationInDays = durationInHours / hoursInDay;
            if (durationInDays < daysInMonth)
            {
                return $"{Math.Round(durationInDays, 1)} days";
            }
            double durationInMonths = durationInDays / daysInMonth;
            if (durationInMonths < monthsInYear)
            {
                return $"{Math.Round(durationInMonths, 1)} months";
            }
            double durationInYears = durationInMonths / monthsInYear;
            return $"{Math.Round(durationInYears, 1)} years";
        }

        /// <summary>
        /// Converts a timezone (+08:00) in string form to parsed timespan
        /// returns null if fail
        /// </summary>
        public static TimeSpan? StringToTimezone(string timezoneRaw)
        {
            try
            {
                return DateTimeOffset.ParseExact(timezoneRaw, "zzz", CultureInfo.InvariantCulture).Offset;
            }
            catch (Exception e)
            {
                return null;
            }
        }

            /// <summary>
        /// Subtracts the specified number of hours from the given DateTimeOffset value.
        /// </summary>
        /// <param name="value">The original DateTimeOffset value.</param>
        /// <param name="hours">The number of hours to subtract.</param>
        /// <returns>A new DateTimeOffset value with the specified hours subtracted.</returns>
        public static DateTimeOffset RemoveHours(this DateTimeOffset value, double hours)
        {
            return value.AddHours(-hours);
        }

        /// <summary>
        /// Remap from 1 range to another
        /// </summary>
        public static float Remap(this float from, float fromMin, float fromMax, float toMin, float toMax)
        {
            var fromAbs = from - fromMin;
            var fromMaxAbs = fromMax - fromMin;

            var normal = fromAbs / fromMaxAbs;

            var toMaxAbs = toMax - toMin;
            var toAbs = toMaxAbs * normal;

            var to = toAbs + toMin;

            return to;
        }

        /// <summary>
        /// Remap from 1 range to another
        /// </summary>
        public static double Remap(this double from, double fromMin, double fromMax, double toMin, double toMax)
        {
            var fromAbs = from - fromMin;
            var fromMaxAbs = fromMax - fromMin;

            var normal = fromAbs / fromMaxAbs;

            var toMaxAbs = toMax - toMin;
            var toAbs = toMaxAbs * normal;

            var to = toAbs + toMin;

            return to;
        }


    }
}
