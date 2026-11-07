# Ondertekening van releases

De releaseworkflow start alleen voor een nieuwe `v*`-tag.
Een eerste taak controleert of de commit al in `main` zit.
Alle platformtaken bouwen daarna dezelfde gecontroleerde commit.
De platformtaken behouden de bestaande ondertekening en controles.
De handmatige herstelroute is verwijderd.
De oude workflow voor herstel van macOS-handtekeningen is verwijderd.
Alle externe actions in de releaseworkflow staan op een volledige commit.

## Grens van deze wijziging

Deze workflowcontrole beschermt de bestaande releaseprocedure.
Hij beschermt organisatiesleutels niet tegen iemand die workflows mag wijzigen.
Een schrijver kan op een eigen branch een andere workflow toevoegen.
Repositorysecrets en toegestane organisatiesecrets blijven daar beschikbaar.
De bescherming van `main` voorkomt dat niet.

Voor die harde grens moeten de sleutels buiten zulke workflows blijven.
Een beheerder kan ze bijvoorbeeld alleen in een beschermde environment beschikbaar maken.
Die environment moet alleen goedgekeurde releasebronnen toelaten.
Dezelfde sleutels mogen dan niet daarnaast als repository- of organisatiesecret beschikbaar blijven.
Alleen een verwijzing naar een environment in YAML toevoegen is niet voldoende.

Deze wijziging verandert geen secrets, environments, rechten of repositoryregels.
Daarom blijft de harde toegangsgrens een open blokkade voor een veilige release.

Bronnen: [GitHub over toegang tot secrets](https://docs.github.com/en/actions/reference/security/secure-use),
[GitHub over environments](https://docs.github.com/en/actions/reference/workflows-and-actions/deployments-and-environments).
