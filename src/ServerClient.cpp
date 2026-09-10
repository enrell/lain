#include "ServerClient.h"

ServerClient::ServerClient(QObject *parent) : QObject(parent) {}

static QVariantMap mockMedia(const QString &id, const QString &title,
                             const QString &year, double progress,
                             const QString &accent, const QString &genre,
                             double rating = 8.0) {
    return {{"id", id},
            {"title", title},
            {"year", year},
            {"runtime", "2h 35m"},
            {"quality", "4K HDR"},
            {"progress", progress},
            {"accent", accent},
            {"genre", genre},
            {"rating", rating},
            {"overview", "Mock local. Trocar pelo backend quando existir."}};
}

static QVariantMap baseFor(const QString &id) {
    if (id == "br2049")
        return mockMedia(id, "Blade Runner 2049", "2017", 0.15, "#E8641F", "Sci-Fi");
    if (id == "alien")
        return mockMedia(id, "Alien", "1979", 0.70, "#7FA88B", "Horror");
    if (id == "heat")
        return mockMedia(id, "Heat", "1995", 0.0, "#5B8DD9", "Crime");
    if (id == "arrival")
        return mockMedia(id, "Arrival", "2016", 0.0, "#4FA3A3", "Sci-Fi");
    if (id == "batman")
        return mockMedia(id, "The Batman", "2022", 0.0, "#B33A3A", "Crime");
    if (id == "spider")
        return mockMedia(id, "Spider-Verse", "2023", 0.0, "#D94F70", "Animation", 8.6);
    if (id == "interstellar")
        return mockMedia(id, "Interstellar", "2014", 0.0, "#4F7FA3", "Sci-Fi", 8.7);
    if (id == "godfather")
        return mockMedia(id, "The Godfather", "1972", 0.0, "#8A7A5B", "Crime", 9.2);
    if (id == "matrix")
        return mockMedia(id, "The Matrix", "1999", 0.0, "#3FA34D", "Sci-Fi", 8.7);
    if (id == "pulp")
        return mockMedia(id, "Pulp Fiction", "1994", 0.0, "#C9A227", "Crime", 8.9);
    if (id == "shining")
        return mockMedia(id, "The Shining", "1980", 0.0, "#7A3B4F", "Horror", 8.4);
    return mockMedia("dune", "Dune", "2021", 0.42, "#D9A45B", "Sci-Fi", 8.0);
}

QVariantMap ServerClient::home() {
    QVariantList cont, recent;
    cont << baseFor("dune") << baseFor("br2049") << baseFor("alien");
    recent << baseFor("heat") << baseFor("arrival") << baseFor("batman") << baseFor("spider");
    return {{"hero", baseFor("dune")},
            {"continueWatching", cont},
            {"recentlyAdded", recent}};
}

QVariantMap ServerClient::media(const QString &id) {
    QVariantMap m = baseFor(id);
    m["overview"] = "Paul Atreides, a brilliant young man born into a great destiny, "
                    "must travel to the most dangerous planet in the universe to ensure "
                    "the future of his family and his people.";
    m["director"] = "Denis Villeneuve";
    m["genres"] = "Sci-Fi · Adventure";

    QVariantList cast;
    const QStringList names = {"Timothée Chalamet", "Rebecca Ferguson", "Oscar Isaac",
                               "Zendaya", "Jason Momoa", "Stellan Skarsgård"};
    const QStringList roles = {"Paul Atreides", "Lady Jessica", "Duke Leto",
                               "Chani", "Duncan Idaho", "Baron Harkonnen"};
    for (int i = 0; i < names.size(); ++i)
        cast << QVariantMap{{"name", names[i]}, {"role", roles[i]}};
    m["cast"] = cast;

    QVariantList related;
    related << baseFor("br2049") << baseFor("alien") << baseFor("arrival") << baseFor("heat");
    m["related"] = related;

    const QString accent = m["accent"].toString();
    QVariantList extras;
    extras << mockMedia("t1", "Trailer", "2021", 0.0, accent, m["genre"].toString())
           << mockMedia("t2", "Behind the Scenes", "2021", 0.0, accent, m["genre"].toString())
           << mockMedia("t3", "Deleted Scenes", "2021", 0.0, accent, m["genre"].toString());
    m["extras"] = extras;

    m["tech"] = QVariantMap{{"video", "HEVC Main 10 · 3840×2160 · HDR10"},
                            {"audio", "TrueHD Atmos 7.1 · English"},
                            {"container", "MKV · 61.2 GB"}};
    return m;
}

QVariantList ServerClient::movies() {
    QVariantList list;
    for (const QString &id : {"dune", "br2049", "alien", "heat", "arrival", "batman",
                              "spider", "interstellar", "godfather", "matrix", "pulp", "shining"})
        list << baseFor(id);
    return list;
}

QString ServerClient::streamUrl(const QString &) {
    return {};
}
