"""Annotated species name -> Perch / BirdNET+ class index.

Rule 1: exact Latin-binomial match.
Rule 2: a hand-written synonym (same biological species, different genus placement or spelling in the
        annotation's taxonomy vs. Perch's).  These are INFERRED from the author's taxonomic knowledge; the only thing
        checked by code is that the target name exists in the Perch list.  Each is listed in results/taxon_synonyms_used.csv.
Rule 3: otherwise the species has NO class (reported as such, never counted as a miss).
A same-genus *different species* is never substituted.
"""
SYNONYMS = {
    # annotation name            : (name in Perch list,           reason)
    "Accipiter cooperii":          ("Astur cooperii",             "Cooper's Hawk, genus placement"),
    "Accipiter tachiro":           ("Aerospiza tachiro",          "African Goshawk, genus placement"),
    "Milvago chimachima":          ("Daptrius chimachima",        "Yellow-headed Caracara, genus placement"),
    "Psilorhinus morio":           ("Cyanocorax morio",           "Brown Jay, genus placement"),
    "Buphagus erythrorynchus":     ("Buphagus erythroryncha",     "Red-billed Oxpecker, spelling of epithet"),
    "Chloropicus fuscescens":      ("Dendropicos fuscescens",     "Cardinal Woodpecker, genus placement"),
    "Corythaixoides leucogaster":  ("Crinifer leucogaster",       "White-bellied Go-away-bird, genus placement"),
    "Cossypha caffra":             ("Dessonornis caffer",         "Cape Robin-Chat, genus placement and gender ending"),
    "Streptopelia senegalensis":   ("Spilopelia senegalensis",    "Laughing Dove, genus placement"),
    "Telophorus nigrifrons":       ("Chlorophoneus nigrifrons",   "Black-fronted Bushshrike, genus placement"),
    "Telophorus sulfureopectus":   ("Chlorophoneus sulfureopectus", "Sulphur-breasted Bushshrike, genus placement"),
    "Leuconotopicus villosus":     ("Dryobates villosus",         "Hairy Woodpecker, genus placement"),
    "Picoides pubescens":          ("Dryobates pubescens",        "Downy Woodpecker, genus placement"),
    "Regulus calendula":           ("Corthylio calendula",        "Ruby-crowned Kinglet, genus placement"),
    "Phylloscopus sibillatrix":    ("Phylloscopus sibilatrix",    "Wood Warbler, misspelling in the CEB table"),
}
UNKNOWN_NAMES = {"unknown", "????", "Aves", ""}


class Mapper:
    def __init__(self, perch_sci, perch_is_species, bnp_sci):
        self.perch = {s: i for i, s in enumerate(perch_sci) if perch_is_species[i]}
        self.bnp = {s: i for i, s in enumerate(bnp_sci)}

    def perch_idx(self, name):
        """-> (idx or None, basis) with basis in exact|synonym|none|unknown"""
        if name in UNKNOWN_NAMES:
            return None, "unknown"
        if name in self.perch:
            return self.perch[name], "exact"
        t = SYNONYMS.get(name)
        if t and t[0] in self.perch:
            return self.perch[t[0]], "synonym"
        return None, "none"

    def bnp_idx(self, name):
        if name in UNKNOWN_NAMES:
            return None
        if name in self.bnp:
            return self.bnp[name]
        t = SYNONYMS.get(name)
        if t and t[0] in self.bnp:
            return self.bnp[t[0]]
        return None
