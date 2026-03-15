module Main exposing (main)

import Browser
import Html exposing (..)
import Html.Attributes as Attr exposing (attribute, class, disabled, id, placeholder, selected, type_, value)
import Html.Events as Events exposing (onClick, onInput)
import Http
import Json.Decode as D exposing (Decoder)
import Url.Builder as Url



-- MAIN


main : Program () Model Msg
main =
    Browser.element
        { init = init
        , update = update
        , subscriptions = \_ -> Sub.none
        , view = view
        }



-- TYPES


type RemoteData a
    = Loading
    | Loaded a
    | Failed String


type SortDir
    = Asc
    | Desc


type alias Sort =
    { column : Int
    , dir : SortDir
    }


type alias Filter =
    { domain : String
    , secondary : String
    }


type PageSize
    = PerPage Int
    | ShowAll


type alias TableState =
    { open : Bool
    , filter : Filter
    , page : Int
    , pageSize : PageSize
    , sort : Maybe Sort
    }


type alias Classification =
    { domain : String
    , classificationType : String
    , isMatchingSite : Bool
    , confidence : Float
    , reasoning : Maybe String
    , model : String
    , validOn : String
    , validUntil : String
    , createdAt : String
    }


type alias ClassificationError =
    { domain : String
    , classificationType : String
    , errorMessage : Maybe String
    , erroredAt : String
    }


type alias StatusMsg =
    { message : String
    , isError : Bool
    }


type alias Model =
    { classifications : RemoteData (List Classification)
    , errors : RemoteData (List ClassificationError)
    , classTable : TableState
    , errTable : TableState
    , status : Maybe StatusMsg
    }


type FilterField
    = DomainField
    | SecondaryField


initTable : TableState
initTable =
    { open = True
    , filter = { domain = "", secondary = "" }
    , page = 1
    , pageSize = PerPage 25
    , sort = Nothing
    }


init : () -> ( Model, Cmd Msg )
init _ =
    ( { classifications = Loading
      , errors = Loading
      , classTable = initTable
      , errTable = initTable
      , status = Nothing
      }
    , Cmd.batch [ fetchClassifications, fetchErrors ]
    )



-- MESSAGES


type Msg
    = GotClassifications (Result Http.Error (List Classification))
    | GotErrors (Result Http.Error (List ClassificationError))
    | ClassFilter FilterField String
    | ErrFilter FilterField String
    | ClassPage Int
    | ErrPage Int
    | ClassPageSize PageSize
    | ErrPageSize PageSize
    | ToggleClass
    | ToggleErr
    | ClassSort Int
    | ErrSort Int
    | ExpireDomain String
    | GotExpire String (Result Http.Error String)
    | RequeueDomain String String
    | GotRequeue String String (Result Http.Error String)
    | RequeueType String
    | GotRequeueType String (Result Http.Error String)
    | RequeueAll
    | GotRequeueAll (Result Http.Error String)
    | DismissStatus



-- UPDATE


update : Msg -> Model -> ( Model, Cmd Msg )
update msg model =
    case msg of
        GotClassifications result ->
            ( { model | classifications = fromHttpResult result }, Cmd.none )

        GotErrors result ->
            ( { model | errors = fromHttpResult result }, Cmd.none )

        ClassFilter field value ->
            ( mapClassTable model (updateFilter field value), Cmd.none )

        ErrFilter field value ->
            ( mapErrTable model (updateFilter field value), Cmd.none )

        ClassPage p ->
            ( mapClassTable model (\t -> { t | page = p }), Cmd.none )

        ErrPage p ->
            ( mapErrTable model (\t -> { t | page = p }), Cmd.none )

        ClassPageSize ps ->
            ( mapClassTable model (\t -> { t | pageSize = ps, page = 1 }), Cmd.none )

        ErrPageSize ps ->
            ( mapErrTable model (\t -> { t | pageSize = ps, page = 1 }), Cmd.none )

        ToggleClass ->
            ( mapClassTable model (\t -> { t | open = not t.open }), Cmd.none )

        ToggleErr ->
            ( mapErrTable model (\t -> { t | open = not t.open }), Cmd.none )

        ClassSort col ->
            ( mapClassTable model (\t -> { t | sort = cycleSort t.sort col, page = 1 }), Cmd.none )

        ErrSort col ->
            ( mapErrTable model (\t -> { t | sort = cycleSort t.sort col, page = 1 }), Cmd.none )

        ExpireDomain domain ->
            ( model
            , postEmpty
                (Url.absolute [ "expire" ] [ Url.string "domain" domain ])
                (GotExpire domain)
            )

        GotExpire domain result ->
            handleAction model ("Expired: " ++ domain) result
                |> withReload

        RequeueDomain domain ct ->
            ( model
            , postEmpty
                (Url.absolute [ "requeue" ]
                    [ Url.string "domain" domain
                    , Url.string "classification_type" ct
                    ]
                )
                (GotRequeue domain ct)
            )

        GotRequeue domain _ result ->
            handleAction model ("Requeued: " ++ domain) result
                |> withReload

        RequeueType ct ->
            ( model
            , postEmpty
                (Url.absolute [ "requeue", "type" ]
                    [ Url.string "classification_type" ct ]
                )
                (GotRequeueType ct)
            )

        GotRequeueType ct result ->
            handleAction model ("Requeued all " ++ ct ++ " errors") result
                |> withReload

        RequeueAll ->
            ( model
            , postEmpty
                (Url.absolute [ "requeue", "all" ] [])
                GotRequeueAll
            )

        GotRequeueAll result ->
            handleAction model "Requeued all errors" result
                |> withReload

        DismissStatus ->
            ( { model | status = Nothing }, Cmd.none )



-- HELPERS


fromHttpResult : Result Http.Error (List a) -> RemoteData (List a)
fromHttpResult result =
    case result of
        Ok xs ->
            Loaded xs

        Err e ->
            Failed (httpErrorToString e)


mapClassTable : Model -> (TableState -> TableState) -> Model
mapClassTable model f =
    { model | classTable = f model.classTable }


mapErrTable : Model -> (TableState -> TableState) -> Model
mapErrTable model f =
    { model | errTable = f model.errTable }


updateFilter : FilterField -> String -> TableState -> TableState
updateFilter field value t =
    let
        f =
            t.filter

        newFilter =
            case field of
                DomainField ->
                    { f | domain = value }

                SecondaryField ->
                    { f | secondary = value }
    in
    { t | filter = newFilter, page = 1 }


cycleSort : Maybe Sort -> Int -> Maybe Sort
cycleSort current col =
    case current of
        Nothing ->
            Just { column = col, dir = Asc }

        Just s ->
            if s.column == col then
                Just { s | dir = flipDir s.dir }

            else
                Just { column = col, dir = Asc }


flipDir : SortDir -> SortDir
flipDir d =
    case d of
        Asc ->
            Desc

        Desc ->
            Asc


handleAction : Model -> String -> Result Http.Error String -> ( Model, Cmd Msg )
handleAction model successText result =
    case result of
        Ok _ ->
            ( { model | status = Just { message = successText, isError = False } }
            , Cmd.none
            )

        Err e ->
            ( { model | status = Just { message = httpErrorToString e, isError = True } }
            , Cmd.none
            )


withReload : ( Model, Cmd Msg ) -> ( Model, Cmd Msg )
withReload ( model, cmd ) =
    ( model, Cmd.batch [ cmd, fetchClassifications, fetchErrors ] )


postEmpty : String -> (Result Http.Error String -> Msg) -> Cmd Msg
postEmpty url toMsg =
    Http.post { url = url, body = Http.emptyBody, expect = Http.expectString toMsg }


httpErrorToString : Http.Error -> String
httpErrorToString err =
    case err of
        Http.BadUrl u ->
            "Bad URL: " ++ u

        Http.Timeout ->
            "Request timed out"

        Http.NetworkError ->
            "Network error"

        Http.BadStatus code ->
            "HTTP error " ++ String.fromInt code

        Http.BadBody body ->
            "Unexpected response: " ++ body


type alias Paged a =
    { visible : List a
    , page : Int
    , pageCount : Int
    , total : Int
    }


paginate : TableState -> List a -> Paged a
paginate ts xs =
    let
        total =
            List.length xs

        pageCount =
            case ts.pageSize of
                ShowAll ->
                    1

                PerPage n ->
                    max 1 (ceiling (toFloat total / toFloat n))

        page =
            clamp 1 pageCount ts.page

        ( start, count ) =
            case ts.pageSize of
                ShowAll ->
                    ( 0, total )

                PerPage n ->
                    ( (page - 1) * n, n )
    in
    { visible = xs |> List.drop start |> List.take count
    , page = page
    , pageCount = pageCount
    , total = total
    }


filterClassifications : Filter -> List Classification -> List Classification
filterClassifications f xs =
    let
        domainTerm =
            String.toLower f.domain

        secTerm =
            String.toLower f.secondary
    in
    List.filter
        (\c ->
            String.contains domainTerm (String.toLower c.domain)
                && String.contains secTerm (String.toLower (Maybe.withDefault "" c.reasoning))
        )
        xs


filterErrors : Filter -> List ClassificationError -> List ClassificationError
filterErrors f xs =
    let
        domainTerm =
            String.toLower f.domain

        secTerm =
            String.toLower f.secondary
    in
    List.filter
        (\e ->
            String.contains domainTerm (String.toLower e.domain)
                && String.contains secTerm (String.toLower (Maybe.withDefault "" e.errorMessage))
        )
        xs


sortClassifications : Maybe Sort -> List Classification -> List Classification
sortClassifications maybeSort xs =
    case maybeSort of
        Nothing ->
            xs

        Just { column, dir } ->
            let
                cmp a b =
                    case column of
                        0 ->
                            compare a.domain b.domain

                        1 ->
                            compare a.classificationType b.classificationType

                        2 ->
                            compare (boolOrd a.isMatchingSite) (boolOrd b.isMatchingSite)

                        3 ->
                            compare a.confidence b.confidence

                        4 ->
                            compare (Maybe.withDefault "" a.reasoning) (Maybe.withDefault "" b.reasoning)

                        5 ->
                            compare a.model b.model

                        6 ->
                            compare a.validOn b.validOn

                        7 ->
                            compare a.validUntil b.validUntil

                        8 ->
                            compare a.createdAt b.createdAt

                        _ ->
                            EQ

                sorted =
                    List.sortWith cmp xs
            in
            case dir of
                Asc ->
                    sorted

                Desc ->
                    List.reverse sorted


sortErrors : Maybe Sort -> List ClassificationError -> List ClassificationError
sortErrors maybeSort xs =
    case maybeSort of
        Nothing ->
            xs

        Just { column, dir } ->
            let
                cmp a b =
                    case column of
                        0 ->
                            compare a.domain b.domain

                        1 ->
                            compare a.classificationType b.classificationType

                        2 ->
                            compare (Maybe.withDefault "" a.errorMessage) (Maybe.withDefault "" b.errorMessage)

                        3 ->
                            compare a.erroredAt b.erroredAt

                        _ ->
                            EQ

                sorted =
                    List.sortWith cmp xs
            in
            case dir of
                Asc ->
                    sorted

                Desc ->
                    List.reverse sorted


boolOrd : Bool -> Int
boolOrd b =
    if b then
        1

    else
        0


uniqueClassificationTypes : List ClassificationError -> List String
uniqueClassificationTypes errors =
    List.foldl
        (\e acc ->
            if List.member e.classificationType acc then
                acc

            else
                acc ++ [ e.classificationType ]
        )
        []
        errors


formatConfidence : Float -> String
formatConfidence f =
    let
        pct =
            round (f * 100)

        whole =
            pct // 100

        frac =
            modBy 100 pct
    in
    String.fromInt whole ++ "." ++ String.padLeft 2 '0' (String.fromInt frac)



-- HTTP


fetchClassifications : Cmd Msg
fetchClassifications =
    Http.get
        { url = Url.absolute [ "classifications" ] []
        , expect = Http.expectJson GotClassifications (D.list classificationDecoder)
        }


fetchErrors : Cmd Msg
fetchErrors =
    Http.get
        { url = Url.absolute [ "errors" ] []
        , expect = Http.expectJson GotErrors (D.list errorDecoder)
        }


andMap : Decoder a -> Decoder (a -> b) -> Decoder b
andMap =
    D.map2 (|>)


classificationDecoder : Decoder Classification
classificationDecoder =
    D.succeed Classification
        |> andMap (D.field "domain" D.string)
        |> andMap (D.field "classification_type" D.string)
        |> andMap (D.field "is_matching_site" D.bool)
        |> andMap (D.field "confidence" D.float)
        |> andMap (D.field "reasoning" (D.nullable D.string))
        |> andMap (D.field "model" D.string)
        |> andMap (D.field "valid_on" D.string)
        |> andMap (D.field "valid_until" D.string)
        |> andMap (D.field "created_at" D.string)


errorDecoder : Decoder ClassificationError
errorDecoder =
    D.succeed ClassificationError
        |> andMap (D.field "domain" D.string)
        |> andMap (D.field "classification_type" D.string)
        |> andMap (D.field "error_message" (D.nullable D.string))
        |> andMap (D.field "errored_at" D.string)



-- VIEW


view : Model -> Html Msg
view model =
    div []
        [ h1 [] [ text "Classifications" ]
        , viewStatus model.status
        , viewErrorsSection model
        , viewClassificationsSection model
        ]


viewStatus : Maybe StatusMsg -> Html Msg
viewStatus maybeStatus =
    case maybeStatus of
        Nothing ->
            text ""

        Just s ->
            div
                [ class
                    (if s.isError then
                        "status-banner status-error"

                     else
                        "status-banner status-ok"
                    )
                ]
                [ span [] [ text s.message ]
                , button [ class "dismiss-btn", onClick DismissStatus ] [ text "×" ]
                ]


viewErrorsSection : Model -> Html Msg
viewErrorsSection model =
    case model.errors of
        Loading ->
            div [ class "section" ] [ p [ class "loading" ] [ text "Loading errors…" ] ]

        Failed err ->
            div [ class "section section-failed" ] [ text ("Failed to load errors: " ++ err) ]

        Loaded errors ->
            let
                filtered =
                    filterErrors model.errTable.filter errors

                sorted =
                    sortErrors model.errTable.sort filtered

                paged =
                    paginate model.errTable sorted

                types =
                    uniqueClassificationTypes errors
            in
            details
                (class "section" :: openAttr model.errTable.open)
                [ summary
                    [ class "section-summary"
                    , preventDefaultClick ToggleErr
                    ]
                    [ text "Errors "
                    , span [ class "section-badge" ] [ text (String.fromInt (List.length errors)) ]
                    ]
                , if model.errTable.open then
                    div [ class "section-body" ]
                        [ div [ class "section-controls" ]
                            [ div [ class "admin-actions" ]
                                (button
                                    [ class "requeue-btn requeue-all-btn"
                                    , onClick RequeueAll
                                    ]
                                    [ text ("Requeue all errors (" ++ String.fromInt (List.length errors) ++ ")") ]
                                    :: List.map (viewRequeueTypeButton errors) types
                                )
                            , div [ class "search-bar" ]
                                [ input
                                    [ type_ "search"
                                    , placeholder "Search domain…"
                                    , value model.errTable.filter.domain
                                    , onInput (ErrFilter DomainField)
                                    ]
                                    []
                                , input
                                    [ type_ "search"
                                    , placeholder "Search error…"
                                    , value model.errTable.filter.secondary
                                    , onInput (ErrFilter SecondaryField)
                                    ]
                                    []
                                ]
                            ]
                        , table [ id "errorsTable" ]
                            [ thead []
                                [ tr []
                                    [ thSort ErrSort 0 model.errTable.sort "Domain"
                                    , thSort ErrSort 1 model.errTable.sort "Type"
                                    , thSort ErrSort 2 model.errTable.sort "Error"
                                    , thSort ErrSort 3 model.errTable.sort "Errored At"
                                    , th [] [ text "Actions" ]
                                    ]
                                ]
                            , tbody [] (List.map viewErrorRow paged.visible)
                            ]
                        , viewPagination paged ErrPage ErrPageSize
                        ]

                  else
                    text ""
                ]


viewRequeueTypeButton : List ClassificationError -> String -> Html Msg
viewRequeueTypeButton errors ct =
    let
        count =
            List.length (List.filter (\e -> e.classificationType == ct) errors)
    in
    button
        [ class "requeue-btn"
        , onClick (RequeueType ct)
        ]
        [ text ("Requeue " ++ ct ++ " errors (" ++ String.fromInt count ++ ")") ]


viewErrorRow : ClassificationError -> Html Msg
viewErrorRow e =
    tr [ class "error-row" ]
        [ td [] [ text e.domain ]
        , td [] [ text e.classificationType ]
        , td [ class "reasoning" ] [ text (Maybe.withDefault "" e.errorMessage) ]
        , td [] [ text e.erroredAt ]
        , td []
            [ button
                [ class "requeue-btn"
                , onClick (RequeueDomain e.domain e.classificationType)
                ]
                [ text "Requeue" ]
            ]
        ]


viewClassificationsSection : Model -> Html Msg
viewClassificationsSection model =
    case model.classifications of
        Loading ->
            div [ class "section" ] [ p [ class "loading" ] [ text "Loading classifications…" ] ]

        Failed err ->
            div [ class "section section-failed" ] [ text ("Failed to load classifications: " ++ err) ]

        Loaded classifications ->
            let
                filtered =
                    filterClassifications model.classTable.filter classifications

                sorted =
                    sortClassifications model.classTable.sort filtered

                paged =
                    paginate model.classTable sorted
            in
            details
                (class "section" :: openAttr model.classTable.open)
                [ summary
                    [ class "section-summary"
                    , preventDefaultClick ToggleClass
                    ]
                    [ text "Classifications "
                    , span [ class "section-badge" ] [ text (String.fromInt (List.length classifications)) ]
                    ]
                , if model.classTable.open then
                    div [ class "section-body" ]
                        [ div [ class "search-bar" ]
                            [ input
                                [ type_ "search"
                                , placeholder "Search domain…"
                                , value model.classTable.filter.domain
                                , onInput (ClassFilter DomainField)
                                ]
                                []
                            , input
                                [ type_ "search"
                                , placeholder "Search reasoning…"
                                , value model.classTable.filter.secondary
                                , onInput (ClassFilter SecondaryField)
                                ]
                                []
                            ]
                        , table [ id "classificationsTable" ]
                            [ thead []
                                [ tr []
                                    [ thSort ClassSort 0 model.classTable.sort "Domain"
                                    , thSort ClassSort 1 model.classTable.sort "Type"
                                    , thSort ClassSort 2 model.classTable.sort "Match"
                                    , thSort ClassSort 3 model.classTable.sort "Confidence"
                                    , thSort ClassSort 4 model.classTable.sort "Reasoning"
                                    , thSort ClassSort 5 model.classTable.sort "Model"
                                    , thSort ClassSort 6 model.classTable.sort "Valid On"
                                    , thSort ClassSort 7 model.classTable.sort "Valid Until"
                                    , thSort ClassSort 8 model.classTable.sort "Created At"
                                    , th [] [ text "Actions" ]
                                    ]
                                ]
                            , tbody [] (List.map viewClassificationRow paged.visible)
                            ]
                        , viewPagination paged ClassPage ClassPageSize
                        ]

                  else
                    text ""
                ]


viewClassificationRow : Classification -> Html Msg
viewClassificationRow c =
    tr []
        [ td [] [ text c.domain ]
        , td [] [ text c.classificationType ]
        , td [] [ text (if c.isMatchingSite then "Yes" else "No") ]
        , td [] [ text (formatConfidence c.confidence) ]
        , td [ class "reasoning" ] [ text (Maybe.withDefault "" c.reasoning) ]
        , td [] [ text c.model ]
        , td [] [ text c.validOn ]
        , td [] [ text c.validUntil ]
        , td [] [ text c.createdAt ]
        , td []
            [ button
                [ class "expire-btn"
                , onClick (ExpireDomain c.domain)
                ]
                [ text "Expire" ]
            ]
        ]


thSort : (Int -> Msg) -> Int -> Maybe Sort -> String -> Html Msg
thSort toMsg col activeSort label =
    let
        indicator =
            case activeSort of
                Just s ->
                    if s.column == col then
                        case s.dir of
                            Asc ->
                                " ▲"

                            Desc ->
                                " ▼"

                    else
                        ""

                Nothing ->
                    ""
    in
    th [ onClick (toMsg col) ] [ text (label ++ indicator) ]


viewPagination : Paged a -> (Int -> Msg) -> (PageSize -> Msg) -> Html Msg
viewPagination paged toPageMsg toPageSizeMsg =
    let
        { visible, page, pageCount, total } =
            paged

        start =
            if total == 0 then
                0

            else
                (page - 1) * List.length visible + 1

        end =
            (page - 1) * List.length visible + List.length visible
    in
    div [ class "pagination-controls" ]
        [ span [ class "page-info" ]
            [ text
                (String.fromInt start
                    ++ "–"
                    ++ String.fromInt end
                    ++ " of "
                    ++ String.fromInt total
                )
            ]
        , select
            [ class "page-size-select"
            , onInput
                (\s ->
                    case s of
                        "all" ->
                            toPageSizeMsg ShowAll

                        n ->
                            toPageSizeMsg (PerPage (Maybe.withDefault 25 (String.toInt n)))
                )
            ]
            [ option [ value "25" ] [ text "25 per page" ]
            , option [ value "50" ] [ text "50 per page" ]
            , option [ value "100" ] [ text "100 per page" ]
            , option [ value "all" ] [ text "All" ]
            ]
        , button
            [ class "page-btn"
            , onClick (toPageMsg (page - 1))
            , disabled (page <= 1)
            ]
            [ text "←" ]
        , span [ class "page-number" ]
            [ text ("Page " ++ String.fromInt page ++ " of " ++ String.fromInt pageCount) ]
        , button
            [ class "page-btn"
            , onClick (toPageMsg (page + 1))
            , disabled (page >= pageCount)
            ]
            [ text "→" ]
        ]


openAttr : Bool -> List (Attribute msg)
openAttr isOpen =
    if isOpen then
        [ attribute "open" "" ]

    else
        []


preventDefaultClick : msg -> Attribute msg
preventDefaultClick msg =
    Events.custom "click"
        (D.succeed
            { message = msg
            , stopPropagation = False
            , preventDefault = True
            }
        )
