"""GitHub: signing in with the device flow of a GitHub App, and asking GitHub in that name.

Columbina keeps no accounts of its own. A user who signs in to GitHub lets Columbina reach
the repositories that the user chose for its app, in the name of the user.

What there is:

- :mod:`columbina.github.app` names the GitHub App and where GitHub is.
- :mod:`columbina.github.store` keeps the tokens, apart from the keys of models.
- :mod:`columbina.github.oauth` gets the tokens with the device flow, and renews them.
- :mod:`columbina.github.session` hands out a token for the API that holds.
- :mod:`columbina.github.client` asks the REST API of GitHub with it.
- :mod:`columbina.github.signing` says who signed in, keeps the sign-in with the picture of
  the user, and lists what Columbina may reach.
"""
