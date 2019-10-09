#pragma once

#include "Core.h"

#include "Vector2.h"
#include "Vector3.h"
#include "Vector4.h"

#include "Quaternion.h"
#include "Matrix4.h"

#include "Transform3.h"
#include "Plane.h"

class GS_API GSM
{
	// +  real
	// += real
	// +  type
	// += type
	// -  real
	// -= real
	// -  type
	// -= type
	// *  real
	// *= real
	// *  type
	// *= type
	// /  real
	// /= real
	// /  type
	// /= type

    static constexpr real SinTable [] =
    {
        0.0,                                //0 deg
        0.01745240643728351281941897851632, //1
        0.03489949670250097164599518162533, //2
        0.05233595624294383272211862960908, //3
        0.06975647374412530077595883519414, //4
        0.08715574274765817355806427083747, //5
        0.1045284632676534713998341548025,  //6
        0.12186934340514748111289391923153, //7
        0.13917310096006544411249666330111, //8
        0.15643446504023086901010531946717, //9
        0.17364817766693034885171662676931, //10
        0.19080899537654481240514048795839, //11
        0.20791169081775933710174228440513, //12
        0.2249510543438649980511072083428,  //13
        0.24192189559966772256044237410035, //14
        0.25881904510252076234889883762405, //15
        0.2756373558169991856499715746113,  //16
        0.29237170472273672809746869537714, //17
        0.30901699437494742410229341718282, //18
        0.32556815445715666871400893579472, //19
        0.34202014332566873304409961468226, //20
        0.35836794954530027348413778941347, //21
        0.3746065934159120354149637745012,  //22
        0.39073112848927375506208458888909, //23
        0.4067366430758002077539859903415,  //24
        0.42261826174069943618697848964773, //25
        0.43837114678907741745273454065827, //26
        0.45399049973954679156040836635787, //27
        0.46947156278589077595946228822784, //28
        0.48480962024633702907537962241578, //29
        0.5,                                //30
        0.51503807491005421008163193639814, //31
        0.52991926423320495404678115181609, //32
        0.54463903501502708222408369208157, //33
        0.55919290347074683016042813998599, //34
        0.57357643635104609610803191282616, //35
        0.58778525229247312916870595463907, //36
        0.60181502315204827991797700044149, //37
        0.61566147532565827966881109284366, //38
        0.62932039104983745270590245827997, //39
        0.64278760968653932632264340990726, //40
        0.65605902899050728478249596402342, //41
        0.66913060635885821382627333068678, //42
        0.68199836006249850044222578471113, //43
        0.69465837045899728665640629942269, //44
        0.70710678118654752440084436210485, //45
        0.71933980033865113935605467445671, //46
        0.73135370161917048328754360827562, //47
        0.74314482547739423501469704897426, //48
        0.75470958022277199794298421956102, //49
        0.76604444311897803520239265055542, //50
        0.7771459614569708799799377436724,  //51
        0.78801075360672195669397778783585, //52
        0.79863551004729284628400080406894, //53
        0.80901699437494742410229341718282, //54
        0.81915204428899178968448838591684, //55
        0.82903757255504169200633684150164, //56
        0.83867056794542402963759094180455, //57
        0.84804809615642597038617617869039, //58
        0.85716730070211228746521798014476, //59
        0.86602540378443864676372317075294, //60
        0.87461970713939580028463695866108, //61
        0.88294759285892694203217136031572, //62
        0.89100652418836786235970957141363, //63
        0.89879404629916699278229567669579, //64
        0.90630778703664996324255265675432, //65
        0.91354545764260089550212757198532, //66
        0.92050485345244032739689472330046, //67
        0.92718385456678740080647445113696, //68
        0.93358042649720174899004306313957, //69
        0.93969262078590838405410927732473, //70
        0.9455185755993168103481247075194,  //71
        0.95105651629515357211643933337938, //72
        0.95630475596303548133865081661842, //73
        0.96126169593831886191649704855706, //74
        0.9659258262890682867497431997289,  //75
        0.97029572627599647230637787403399, //76
        0.97437006478523522853969448008827, //77
        0.9781476007338056379285667478696,  //78
        0.98162718344766395349650489981814, //79
        0.98480775301220805936674302458952, //80
        0.98768834059513772619004024769344, //81
        0.99026806874157031508377486734485, //82
        0.99254615164132203498006158933058, //83
        0.99452189536827333692269194498057, //84
        0.99619469809174553229501040247389, //85
        0.99756405025982424761316268064426, //86
        0.99862953475457387378449205843944, //87
        0.99939082701909573000624344004393, //88
        0.99984769515639123915701155881391, //89
        1.0,                                //90
        0.99984769515639123915701155881391, //89
        0.99939082701909573000624344004393, //88
        0.99862953475457387378449205843944, //87
        0.99756405025982424761316268064426, //86
        0.99619469809174553229501040247389, //85
        0.99452189536827333692269194498057, //84
        0.99254615164132203498006158933058, //83
        0.99026806874157031508377486734485, //82
        0.98768834059513772619004024769344, //81
        0.98480775301220805936674302458952, //80
        0.98162718344766395349650489981814, //79
        0.9781476007338056379285667478696,  //78
        0.97437006478523522853969448008827, //77
        0.97029572627599647230637787403399, //76
        0.9659258262890682867497431997289,  //75
        0.96126169593831886191649704855706, //74
        0.95630475596303548133865081661842, //73
        0.95105651629515357211643933337938, //72
        0.9455185755993168103481247075194,  //71
        0.93969262078590838405410927732473, //70
        0.93358042649720174899004306313957, //69
        0.92718385456678740080647445113696, //68
        0.92050485345244032739689472330046, //67
        0.91354545764260089550212757198532, //66
        0.90630778703664996324255265675432, //65
        0.89879404629916699278229567669579, //64
        0.89100652418836786235970957141363, //63
        0.88294759285892694203217136031572, //62
        0.87461970713939580028463695866108, //61
        0.86602540378443864676372317075294, //60
        0.85716730070211228746521798014476, //59
        0.84804809615642597038617617869039, //58
        0.83867056794542402963759094180455, //57
        0.82903757255504169200633684150164, //56
        0.81915204428899178968448838591684, //55
        0.80901699437494742410229341718282, //54
        0.79863551004729284628400080406894, //53
        0.78801075360672195669397778783585, //52
        0.7771459614569708799799377436724,  //51
        0.76604444311897803520239265055542, //50
        0.75470958022277199794298421956102, //49
        0.74314482547739423501469704897426, //48
        0.73135370161917048328754360827562, //47
        0.71933980033865113935605467445671, //46
        0.70710678118654752440084436210485, //45
        0.69465837045899728665640629942269, //44
        0.68199836006249850044222578471113, //43
        0.66913060635885821382627333068678, //42
        0.65605902899050728478249596402342, //41
        0.64278760968653932632264340990726, //40
        0.62932039104983745270590245827997, //39
        0.61566147532565827966881109284366, //38
        0.60181502315204827991797700044149, //37
        0.58778525229247312916870595463907, //36
        0.57357643635104609610803191282616, //35
        0.55919290347074683016042813998599, //34
        0.54463903501502708222408369208157, //33
        0.52991926423320495404678115181609, //32
        0.51503807491005421008163193639814, //31
        0.5,                                //30
        0.48480962024633702907537962241578, //29
        0.46947156278589077595946228822784, //28
        0.45399049973954679156040836635787, //27
        0.43837114678907741745273454065827, //26
        0.42261826174069943618697848964773, //25
        0.4067366430758002077539859903415,  //24
        0.39073112848927375506208458888909, //23
        0.3746065934159120354149637745012,  //22
        0.35836794954530027348413778941347, //21
        0.34202014332566873304409961468226, //20
        0.32556815445715666871400893579472, //19
        0.30901699437494742410229341718282, //18
        0.29237170472273672809746869537714, //17
        0.2756373558169991856499715746113,  //16
        0.25881904510252076234889883762405, //15
        0.24192189559966772256044237410035, //14
        0.2249510543438649980511072083428,  //13
        0.20791169081775933710174228440513, //12
        0.19080899537654481240514048795839, //11
        0.17364817766693034885171662676931, //10
        0.15643446504023086901010531946717, //9
        0.13917310096006544411249666330111, //8
        0.12186934340514748111289391923153, //7
        0.1045284632676534713998341548025,  //6
        0.08715574274765817355806427083747, //5
        0.06975647374412530077595883519414, //4
        0.05233595624294383272211862960908, //3
        0.03489949670250097164599518162533, //2
        0.01745240643728351281941897851632, //1
    };

    static constexpr real TanTable[] = {
        0.0,                                //0 deg
        0.01745506492821758576512889521973, //1
        0.03492076949174773050040262577373, //2
        0.05240777928304120403880582447398, //3
        0.06992681194351041366692106032318, //4
        0.08748866352592400522201866943496, //5
        0.10510423526567646251150238013988, //6
        0.12278456090290459113423113605286, //7
        0.14054083470239144683811769343281, //8
        0.15838444032453629383888309269437, //9
        0.17632698070846497347109038686862, //10
        0.19438030913771848424319422497682, //11
        0.21255656167002212525959166057008, //12
        0.23086819112556311174814561347445, //13
        0.24932800284318069162403993780486, //14
        0.26794919243112270647255365849413, //15
        0.28674538575880794004275806273267, //16
        0.30573068145866035573454195899655, //17
        0.32491969623290632615587141221513, //18
        0.34432761328966524195726583938311, //19
        0.36397023426620236135104788277683, //20
        0.38386403503541579597144840810327, //21
        0.40402622583515681132234814357991, //22
        0.42447481620960474202353206294252, //23
        0.44522868530853616392236703064567, //24
        0.46630765815499859283000619479956, //25
        0.48773258856586142277311112661696, //26
        0.50952544949442881051370691125066, //27
        0.53170943166147874807591587184006, //28
        0.55430905145276891782076309233813, //29
        0.57735026918962576450914878050196, //30
        0.60086061902756041487866442635466, //31
        0.62486935190932750978051082794944, //32
        0.64940759319751057698206291131145, //33
        0.67450851684242663214246086199461, //34
        0.70020753820970977945852271944483, //35
        0.72654252800536088589546675748062, //36
        0.75355405010279415707395644862159, //37
        0.78128562650671739706294997196227, //38
        0.80978403319500714803699137423577, //39
        0.83909963117728001176312729812318, //40
        0.86928673781622666220009563870394, //41
        0.90040404429783994512047720388537, //42
        0.93251508613766170561218562742619, //43
        0.96568877480707404595802729970068, //44
        1.0,                                //45 deg
        1.0355303137905695069588325512481,  //46
        1.0723687100246825329460277480726,  //47
        1.1106125148291928701434819641651,  //48
        1.1503684072210095558763310255696,  //49
        1.1917535925942099587053080718604,  //50
        1.2348971565350513985561746953759,  //51
        1.279941632193078780311029847572,   //52
        1.3270448216204100371594725740869,  //53
        1.3763819204711735382072095819109,  //54
        1.4281480067421145021606184849985,  //55
        1.4825609685127402547871571491544,  //56
        1.5398649638145829048267969726028,  //57
        1.6003345290410503553267330811834,  //58
        1.6642794823505179110304961700348,  //59
        1.7320508075688772935274463415059,  //60
        1.804047755271423937381784748237,   //61
        1.8807264653463320123608375958293,  //62
        1.9626105055051505823046404262119,  //63
        2.0503038415792962168990110705415,  //64
        2.1445069205095586163562607910459,  //65
        2.2460367739042160541633214384164,  //66
        2.3558523658237528339395866623439,  //67
        2.4750868534162958252400132460762,  //68
        2.6050890646938015362584123364335,  //69
        2.7474774194546222787616640264977,  //70
        2.9042108776758228025793255345271,  //71
        3.0776835371752534025702905760369,  //72
        3.2708526184841408653088562573054,  //73
        3.4874144438409086506962242250994,  //74
        3.7320508075688772935274463415059,  //75
        4.0107809335358447163457151294634,  //76
        4.3314758742841555455461677545574,  //77
        4.7046301094784542335862345374029,  //78
        5.1445540159703101347232207171292,  //79
        5.671281819617709530994418439864,   //80
        6.3137515146750430989794642447682,  //81
        7.1153697223842087482305661436316,  //82
        8.1443464279745940238256613949797,  //83
        9.5143644542225849296839714549457,  //84
        11.430052302761343067210855549163,  //85
        14.300666256711927910128053347586,  //86
        19.081136687728211063406748734365,  //87
        28.636253282915603550756509320946,  //88
        57.289961630759424687278147537113,  //89
        1000.0                              //~90
    };

    static constexpr real ArcSinTable[] = {
        0.0,                               //0.00
        2.8659839825988619870938253798705, //0.05
        5.7391704772667863125149089039304, //0.10
        8.6269265586786377690081747853221, //0.15
        11.536959032815487690137371510513, //0.20
        14.477512185929923878771034799127, //0.25
        17.457603123722092290246045792445, //0.30
        20.487315114722663466756581659438, //0.35
        23.578178478201831104022499419827, //0.40
        26.74368395040300635931349305488,  //0.45
        30.0,                              //0.50
        33.367012969231750012250966510535, //0.55
        36.869897645844021296855612559093, //0.60
        40.541601873504521758912533280828, //0.65
        44.42700400080570372674461224586,  //0.70
        48.590377890729140661519497813074, //0.75
        53.130102354155978703144387440907, //0.80
        58.211669382948380165928340391684, //0.85
        64.15806723683287126483046074049,  //0.90
        71.805127661233223475817833285113, //0.95
        90.0                               //1.00
    };

    static constexpr real AtanTable[] = {
        0.0,                               //0.0
        5.7105931374996425126958813482344, //0.1
        11.30993247402021308647450543834,  //0.2
        16.699244233993621840368847179984, //0.3
        21.801409486351811770244866086944, //0.4
        26.565051177077989351572193720453, //0.5
        30.963756532073521417107679840837, //0.6
        34.992020198558662106922891030458, //0.7
        38.659808254090090604005862335173, //0.8
        41.987212495816660054881945785005, //0.9
        45.0,                              //1.0
        47.726310993906265496417814720824, //1.1
        50.194428907734805993720510180702, //1.2
        52.431407971172507410861476593104, //1.3
        54.462322208025617391140070541742, //1.4
        56.30993247402021308647450543834,  //1.5
        57.994616791916504439209354249595, //1.6
        59.534455080540120523946403127274, //1.7
        60.945395900922854797657689523261, //1.8
        62.241459398939975604142840287841, //1.9
        63.434948822922010648427806279547, //2.0
        64.536654938128385375317560191316, //2.1
        65.556045219583464308293612747344, //2.2
        66.501434324047904025967609299095, //2.3
        67.38013505195957382705098912332,  //2.4
        68.198590513648188229755133913056, //2.5
        68.962488974578183239871829391072, //2.6
        69.676863170337058889045969071538, //2.7
        70.346175941946691669366825597503, //2.8
        70.974393962431318362586899782639, //2.9
        71.565051177077989351572193720453, //3.0
        72.121303404158661783031996253752, //3.1
        72.645975363738677991389154481147, //3.2
        73.141601232261721166239003751771, //3.3
        73.610459665965216957704244177375, //3.4
        74.054604099077145202342310476739, //3.5
        74.475889003245742642893186527887, //3.6
        74.875992691689432718614080928839, //3.7
        75.256437163529264838608545391579, //3.8
        75.618605408909396405399434110072, //3.9
        75.963756532073521417107679840837, //4.0
        76.293038995920191467926695706637, //4.1
        76.607502246248902249774189372904, //4.2
        76.908106935653154740085142815611, //4.3
        77.195733934713249329565613296061, //4.4
        77.47119229084848923132012643871,  //4.5
        77.735226272107598215662979268428, //4.6
        77.988521613634558949655095994244, //4.7
        78.231711067979355456511847585059, //4.8
        78.46537934635528461028535215447,  //4.9
        78.69006752597978691352549456166,  //5.0
        78.906276988442153082846457794398, //5.1
        79.114472945341262256847665545467, //5.2
        79.315087599997283726247799952269, //5.3
        79.508522987668401316229639351396, //5.4
        79.69515353123396805471658116136,  //5.5
        79.87532834460218360920191811979,  //5.6
        80.049373312048397939091982629163, //5.7
        80.217592968192713898721296954961, //5.8
        80.380272200301147138603593974522, //5.9
        80.537677791974382608859929458258, //6.0
        80.69005982501396321613676438502,  //6.1
        80.837652954278290353137186246107, //6.2
        80.980677568618316454083718216286, //6.3
        81.119340849479754594205710230109, //6.4
        81.253837737444791062798919511021, //6.5
        81.384351815835887719627972645329, //6.6
        81.511056119495283774802739379903, //6.7
        81.634113875967410736762152347355, //6.8
        81.753679185531469847774287981663, //6.9
        81.869897645844021296855612559093, //7.0
        81.982906926344671557785860132189, //7.1
        82.092837297041543150574174078939, //7.2
        82.199812115818301554511155931107, //7.3
        82.303948277983430813101817505664, //7.4
        82.405356631408555015724136744871, //7.5
        82.504142360270141108436908194481, //7.6
        82.600405340112889602085198703429, //7.7
        82.694240466689174212640435599989, //7.8
        82.785737960793239699760426352318, //7.9
        82.874983651098202438046699158793, //8.0
        82.962059236815330894379582560045, //8.1
        83.04704253182608430536863945651,  //8.2
        83.130007691785741126288670895299, //8.3
        83.211025425561209127237192801605, //8.4
        83.290163192243066861532898701183, //8.5
        83.36748538486153719839113265321,  //8.6
        83.443053501836611957180340257904, //8.7
        83.516926307102762593482381018481, //8.8
        83.589159979767551813345186139976, //8.9
        83.659808254090090604005862335173, //9.0
        83.728922550498854604460225768617, //9.1
        83.79655209830816478596192287085,  //9.2
        83.862744050738012371659801379595, //9.3
        83.927543592792299914283906222659, //9.4
        83.990994042505474956721419026891, //9.5
        84.053136946026500201440129890867, //9.6
        84.114012166971733639544708372717, //9.7
        84.17365797044422557908366587801,  //9.8
        84.232111102085856878307043277746, //9.9
        84.289406862500357487304118651766, //10.0
        84.34557917735930002240386504446,  //10.1
        84.400660663479429426221375909894, //10.2
        84.454682691137960235501642827555, //10.3
        84.507675442872561976839251643601, //10.4
        84.559667968994493790781547970049, //10.5
        84.610688240026591246105658258356, //10.6
        84.66076319626241600371917906752,  //10.7
        84.709918794628730354308760329816, //10.8
        84.758180053020443389464709991252, //10.9
        84.805571092265194006279489819298, //11.0
        84.85211517586369794981011127492,  //11.1
        84.89783474764181007012398109304,  //11.2
        84.942751467440864985949441651789, //11.3
        84.986886244964192373865427162674, //11.4
        85.030259271889696309519632226488, //11.5
        85.07289005235098622641982952123,  //11.6
        85.114797431882699908615027443976, //11.7
        85.155999624919320811750501469564, //11.8
        85.196514240930921321748681887251, //11.9
        85.23635830927382241867267236649,  //12.0
    };

	inline static uint8 RandUseCount = 0;
	static constexpr uint32 RandTable[] = { 542909189, 241292975, 485392319, 280587594, 22564577, 131346666, 540115444, 163133756, 7684350, 906455780 };
	inline static uint8 FloatRandUseCount = 0;
	static constexpr float FloatRandTable[] = { 0.7406606394, 0.8370865161, 0.3390759540, 0.4997499184, 0.0598975500, 0.1089056913, 0.3401726208, 0.2333399466, 0.3234475486, 0.2359271793 };

	INLINE static float Sin(const float Degrees)
	{
		const uint8 a = Floor(Degrees);

		return Lerp(SinTable[a], SinTable[a + 1], Degrees - a);
	}

	INLINE static float Tan(const float Degrees)
	{
		const uint8 a = Floor(Degrees);

		return Lerp(TanTable[a], TanTable[a + 1], Degrees - a);
	}

	INLINE static float ASin(float Degrees)
	{
		Degrees *= 20.0f;

		const uint8 a = Floor(Degrees);

		return Lerp(ArcSinTable[a], ArcSinTable[a + 1], Degrees - a);
	}

	INLINE static float ACos(float Degrees)
	{
		Degrees *= 20.0f;

		const uint8 a = Floor(Degrees);

		return Lerp(ArcSinTable[a], ArcSinTable[a + 1], Degrees - a);
	}

	INLINE static float ATan(float Degrees)
	{
		Degrees *= 10.0f;

		const uint8 a = Floor(Degrees);

		return Lerp(AtanTable[a], AtanTable[a + 1], Degrees - a);
	}

	INLINE static real StraightRaise(const real A, const uint8 Times)
	{
		real Result = A;

		for (uint8 i = 0; i < Times - 1; i++)
		{
			Result *= A;
		}

		return Result;
	}

public:
	static constexpr double PI = 3.141592653589793238462643383279502884197169399375105820974944592307816406286;
	static constexpr double e =  2.718281828459045235360287471352662497757247093699959574966967627724076630353;

	INLINE static int64 Random()
	{
		int64 ret = RandTable[RandUseCount];

		ret = RandUseCount % 2 == 1 ? ret * -1 : ret;

		RandUseCount = (RandUseCount + 1) % 10;

		return ret;
	}

	INLINE static int64 Random(int64 _Min, int64 _Max)
	{
		return Random() % (_Max - _Min + 1) + _Min;
	}

	INLINE static real fRandom()
	{
		auto ret = FloatRandTable[FloatRandUseCount];

		ret = FloatRandUseCount % 2 == 1 ? ret * -1 : ret;

		RandUseCount = (RandUseCount + 1) % 10;

		return ret;
	}

	//INLINE STATIC

	INLINE static int32 Floor(const float A)
	{
		return static_cast<int32>(A - (static_cast<int32>(A) % 1));
	}

	INLINE static float Modulo(const float A, const float B)
	{
		const float C = A / B;
		return (C - Floor(C)) * B;
	}

	//INLINE static float Power(const float Base, const int32 Exp)
	//{
	//	if (Exp < 0)
	//	{
	//		if (Base == 0)
	//		{
	//			return -0; // Error!!
	//		}
	//
	//		return 1 / (Base * Power(Base, (-Exp) - 1));
	//	}
	//
	//	if (Exp == 0)
	//	{
	//		return 1;
	//	}
	//
	//	if (Exp == 1)
	//	{
	//		return Base;
	//	}
	//
	//	return Base * Power(Base, Exp - 1);
	//}

	INLINE static uint32 Fact(const int8 A)
	{
		uint8 Result = 1;

		for (uint8 i = 1; i < A + 1; i++)
		{
			Result *= (i + 1);
		}

		return Result;
	}

	//Returns the sine of an angle.
	INLINE static float Sine(const float Degrees)
	{
		const float abs = Abs(Degrees);

		if (Modulo(abs, 360.0f) > 180.0f)
		{
			return -Sin(Modulo(abs, 180.0f));
		}
		else
		{
			return Sin(Modulo(abs, 180.0f));
		}
	}

	//Returns the cosine of an angle.
	INLINE static float Cosine(const float Degrees)
	{
		return Sine(Degrees + 90.0f);
	}

	//Returns the tangent of an angle. INPUT DEGREES MUST BE BETWEEN 0 AND 90.
	INLINE static float Tangent(const float Degrees)
	{
		if (Degrees > 0.0f)
		{
			return Tan(Degrees);
		}
		else
		{
			return -Tan(Abs(Degrees));
		}
	}

	//Returns the ArcSine. INPUT DEGREES MUST BE BETWEEN 0 AND 1.
	INLINE static float ArcSine(const float A)
	{
		if (A > 0.0f)
		{
			return ASin(A);
		}
		else
		{
			return -ASin(Abs(A));
		}
	}

	INLINE static float ArcCosine(const float A)
	{
		if (A > 0.0f)
		{
			return 90.0f - ASin(1.0f - A);
		}
		else
		{
			return 90.0f + ASin(Abs(A));
		}
	}

	//Returns the arctangent of the number. INPUT DEGREES MUST BE BETWEEN 0 AND 12.
	INLINE static float ArcTangent(const float A)
	{
		if (A > 0.0f)
		{
			return ATan(A);
		}
		else
		{
			return -ATan(Abs(A));
		}
	}

	INLINE static float ArcTan2(const float X, const float Y)
	{
		return ArcTangent(Y / X);
	}

	INLINE static real Power(const real _A, const real Times)
	{
		const real Timesplus = StraightRaise(_A, Floor(Times));

		return Lerp(Timesplus, Timesplus * _A, Times - Floor(Times));
	}

	//////////////////////////////////////////////////////////////
	//						SCALAR MATH							//
	//////////////////////////////////////////////////////////////

	//Returns 1 if A is bigger than 0. 0 if A is equal to 0. and -1 if A is less than 0.
	INLINE static int8 Sign(const uint_64 _A)
	{
		if (_A > 0)
		{
			return 1;
		}
		else if (_A < 0)
		{
			return -1;
		}
		else
		{
			return 0;
		}
	}

	//Returns 1 if A is bigger than 0. 0 if A is equal to 0. and -1 if A is less than 0.
	INLINE static int8 Sign(const real A)
	{
		if (A > 0.0)
		{
			return 1;
		}
		else if (A < 0.0)
		{
			return -1;
		}
		else
		{
			return 0;
		}
	}

	//Mixes A and B by the specified values, Where Alpha 0 returns A and Alpha 1 returns B.
	INLINE static float Lerp(const float A, const float B, const float Alpha)
	{
		return A + Alpha * (B - A);
	}

	//Interpolates from Current to Target, returns Current + an amount determined by the InterpSpeed.
	INLINE static float FInterp(const float Target, const float Current, const float DT, const float InterpSpeed)
	{
		return (((Target - Current) * DT) * InterpSpeed) + Current;
	}

	INLINE static float MapToRange(const float A, const float InMin, const float InMax, const float OutMin, const float OutMax)
	{
		return InMin + ((OutMax - OutMin) / (InMax - InMin)) * (A - InMin);
	}

	INLINE static float obMapToRange(const float A, const float InMax, const float OutMax)
	{
		return A / (InMax / OutMax);
	}

	INLINE static real SquareRoot(real _A)
	{
		constexpr auto error = 0.00001; //define the precision of your result
		double s = _A;

		while (s - _A / s > error) //loop until precision satisfied 
		{
			s = (s + _A / s) / 2.0;
		}

		return s;
	}

	INLINE static real Root(const real _A, const real _Root)
	{
		return Power(_A, 1.0 / _Root);
	}

	INLINE static uint32 Abs(const int32 A)
	{
		return A > 0 ? A : -A;
	}

	INLINE static float Abs(float _A)
	{
		return _A > 0.0f ? _A : -_A;
	}

	INLINE static int32 Min(const int32 A, const int32 B)
	{
		return (A < B) ? A : B;
	}

	INLINE static int32 Max(const int32 A, const int32 B)
	{
		return (A > B) ? A : B;
	}

	INLINE static float Min(const float A, const float B)
	{
		return (A < B) ? A : B;
	}

	INLINE static float Max(const float A, const float B)
	{
		return (A > B) ? A : B;
	}

	template<typename T>
	INLINE static T Min(const T & A, const T & B)
	{
		return (A < B) ? A : B;
	}

	template<typename T>
	INLINE static T Max(const T & A, const T & B)
	{
		return (A > B) ? A : B;
	}

	INLINE static real DegreesToRadians(const real Degrees)
	{
		return Degrees * PI / 180.0;
	}

	INLINE static real RadiansToDegrees(const real Radians)
	{
		return Radians * 180.0 / PI;
	}

	//////////////////////////////////////////////////////////////
	//						VECTOR MATH							//
	//////////////////////////////////////////////////////////////

	//Calculates the length of a 2D vector.
	INLINE static float VectorLength(const Vector2 & Vec1)
	{
		return SquareRoot(Vec1.X * Vec1.X + Vec1.Y * Vec1.Y);
	}

	INLINE static float VectorLength(const Vector3 & Vec1)
	{
		return SquareRoot(Vec1.X * Vec1.X + Vec1.Y * Vec1.Y + Vec1.Z * Vec1.Z);
	}

	INLINE static float VectorLength(const Vector4 & Vec1)
	{
		return SquareRoot(Vec1.X * Vec1.X + Vec1.Y * Vec1.Y + Vec1.Z * Vec1.Z + Vec1.W * Vec1.W);
	}

	INLINE static float VectorLengthSquared(const Vector2 & Vec1)
	{
		return Vec1.X * Vec1.X + Vec1.Y * Vec1.Y;
	}

	INLINE static float VectorLengthSquared(const Vector3 & Vec1)
	{
		return Vec1.X * Vec1.X + Vec1.Y * Vec1.Y + Vec1.Z * Vec1.Z;
	}

	INLINE static Vector2 Normalized(const Vector2 & Vec1)
	{
		const float Length = VectorLength(Vec1);
		return Vector2(Vec1.X / Length, Vec1.Y / Length);
	}

	INLINE static void Normalize(Vector2 & Vec1)
	{
		const float Length = VectorLength(Vec1);

		Vec1.X = Vec1.X / Length;
		Vec1.Y = Vec1.Y / Length;
	}

	INLINE static Vector3 Normalized(const Vector3 & Vec1)
	{
		const float Length = VectorLength(Vec1);
		return Vector3(Vec1.X / Length, Vec1.Y / Length, Vec1.Z / Length);
	}

	INLINE static void Normalize(Vector3 & Vec1)
	{
		const float Length = VectorLength(Vec1);

		Vec1.X = Vec1.X / Length;
		Vec1.Y = Vec1.Y / Length;
		Vec1.Z = Vec1.Z / Length;
	}

	INLINE static Vector4 Normalized(const Vector4 & Vec1)
	{
		const float Length = VectorLength(Vec1);
		return Vector4(Vec1.X / Length, Vec1.Y / Length, Vec1.Z / Length, Vec1.W / Length);
	}

	INLINE static void Normalize(Vector4 & Vec1)
	{
		const float Length = VectorLength(Vec1);

		Vec1.X = Vec1.X / Length;
		Vec1.Y = Vec1.Y / Length;
		Vec1.Z = Vec1.Z / Length;
		Vec1.W = Vec1.W / Length;
	}

	INLINE static float Dot(const Vector2 & Vec1, const Vector2 & Vec2)
	{
		return Vec1.X * Vec2.X + Vec1.Y * Vec2.Y;
	}

	INLINE static float Dot(const Vector3 & Vec1, const Vector3 & Vec2)
	{
		return Vec1.X * Vec2.X + Vec1.Y * Vec2.Y + Vec1.Z * Vec2.Z;
	}

	INLINE static Vector3 Cross(const Vector3 & Vec1, const Vector3 & Vec2)
	{
		return Vector3(Vec1.Y * Vec2.Z - Vec1.Z * Vec2.Y, Vec1.Z * Vec2.X - Vec1.X * Vec2.Z, Vec1.X * Vec2.Y - Vec1.Y * Vec2.X);
	}

	INLINE static Vector2 AbsVector(const Vector2 & Vec1)
	{
		return Vector2(Abs(Vec1.X), Abs(Vec1.Y));
	}

	INLINE static Vector3 AbsVector(const Vector3 & Vec1)
	{
		return Vector3(Abs(Vec1.X), Abs(Vec1.Y), Abs(Vec1.Z));
	}

	INLINE static Vector2 Negated(const Vector2 & Vec)
	{
		Vector2 Result;

		Result.X = -Vec.X;
		Result.Y = -Vec.Y;

		return Result;
	}

	INLINE static void Negate(Vector2 & Vec)
	{
		Vec.X = -Vec.X;
		Vec.Y = -Vec.Y;

		return;
	}

	INLINE static Vector3 Negated(const Vector3 & Vec)
	{
		Vector3 Result;

		Result.X = -Vec.X;
		Result.Y = -Vec.Y;
		Result.Z = -Vec.Z;

		return Result;
	}

	INLINE static void Negate(Vector3 & Vec)
	{
		Vec.X = -Vec.X;
		Vec.Y = -Vec.Y;
		Vec.Z = -Vec.Z;

		return;
	}

	INLINE static Vector4 Negated(const Vector4 & Vec)
	{
		Vector4 Result;

		Result.X = -Vec.X;
		Result.Y = -Vec.Y;
		Result.Z = -Vec.Z;
		Result.W = -Vec.W;

		return Result;
	}

	INLINE static void Negate(Vector4 & Vec)
	{
		Vec.X = -Vec.X;
		Vec.Y = -Vec.Y;
		Vec.Z = -Vec.Z;
		Vec.W = -Vec.W;

		return;
	}

	//////////////////////////////////////////////////////////////
	//						QUATERNION MATH						//
	//////////////////////////////////////////////////////////////

    INLINE static real Dot(const Quaternion& _A, const Quaternion& _B)
    {
        return _A.X * _B.X + _A.Y * _B.Y + _A.Z * _B.Z + _A.Q * _B.Q;
    }

	INLINE static float Length(const Quaternion& _A)
	{
		return SquareRoot(Dot(_A, _A));
	}

	INLINE static Quaternion Normalized(const Quaternion & Quat)
	{
		const float lLength = Length(Quat);

		return Quaternion(Quat.X / lLength, Quat.Y / lLength, Quat.Z / lLength, Quat.Q / lLength);
	}

	INLINE static void Normalize(Quaternion & Quat)
	{
		const float lLength = Length(Quat);

		Quat.X = Quat.X / lLength;
		Quat.Y = Quat.Y / lLength;
		Quat.Z = Quat.Z / lLength;
		Quat.Q = Quat.Q / lLength;
	}

	INLINE static Quaternion Conjugated(const Quaternion & Quat)
	{
		return Quaternion(-Quat.X, -Quat.Y, -Quat.Z, Quat.Q);
	}

	INLINE static void Conjugate(Quaternion & Quat)
	{
		Quat.X = -Quat.X;
		Quat.Y = -Quat.Y;
		Quat.Z = -Quat.Z;

		return;
	}


	//////////////////////////////////////////////////////////////
	//						LOGIC								//
	//////////////////////////////////////////////////////////////

	INLINE static bool IsNearlyEqual(const float A, const float Target, const float Tolerance)
	{
		return (A > Target - Tolerance) && (A < Target + Tolerance);
	}

	INLINE static bool IsInRange(const float A, const float Min, const float Max)
	{
		return (A > Min) && (A < Max);
	}

	INLINE static bool IsVectorEqual(const Vector2 & A, const Vector2 & B)
	{
		return A.X == B.X && A.Y == B.Y;
	}

	INLINE static bool IsVectorEqual(const Vector3 & A, const Vector3 & B)
	{
		return A.X == B.X && A.Y == B.Y && A.Z == B.Z;
	}

	INLINE static bool IsVectorNearlyEqual(const Vector2 & A, const Vector2 & Target, const float Tolerance)
	{
		return IsNearlyEqual(A.X, Target.X, Tolerance) && IsNearlyEqual(A.Y, Target.Y, Tolerance);
	}

	INLINE static bool IsVectorNearlyEqual(const Vector3 & A, const Vector3 & Target, const float Tolerance)
	{
		return IsNearlyEqual(A.X, Target.X, Tolerance) && IsNearlyEqual(A.Y, Target.Y, Tolerance) && IsNearlyEqual(A.Z, Target.Z, Tolerance);
	}

	INLINE static bool AreVectorComponentsGreater(const Vector3 & A, const Vector3 & B)
	{
		return A.X > B.X && A.Y > B.Y && A.Z > B.Z;
	}

	//////////////////////////////////////////////////////////////
	//						MATRIX MATH							//
	//////////////////////////////////////////////////////////////

	//Creates a translation matrix.
	INLINE static Matrix4 Translation(const Vector3 & Vector)
	{
		Matrix4 Result;

		Result[0 + 3 * 4] = Vector.X;
		Result[1 + 3 * 4] = Vector.Y;
		Result[2 + 3 * 4] = Vector.Z;

		return Result;
	}

	//Modifies the given matrix to make it a translation matrix.
	INLINE static void Translate(Matrix4 & Matrix, const Vector3 & Vector)
	{
		Matrix[0 + 3 * 4] = Vector.X;
		Matrix[1 + 3 * 4] = Vector.Y;
		Matrix[2 + 3 * 4] = Vector.Z;

		return;
	}

	INLINE static void Rotate(Matrix4 & A, const Quaternion & Q)
	{
		const float cos = Cosine(Q.Q);
		const float sin = Sine(Q.Q);
		const float omc = 1.0f - cos;

		A[0] = Q.X * omc + cos;
		A[1] = Q.Y * Q.X * omc - Q.Y * sin;
		A[2] = Q.X * Q.Z * omc - Q.Y * sin;

		A[4] = Q.X * Q.Y * omc - Q.Z * sin;
		A[5] = Q.Y * omc + cos;
		A[6] = Q.Y * Q.Z * omc + Q.X * sin;

		A[8] = Q.X * Q.Z * omc + Q.Y * sin;
		A[9] = Q.Y * Q.Z * omc - Q.X * sin;
		A[10] = Q.Z * omc + cos;
	}

	INLINE static Matrix4 Rotation(const Quaternion & A)
	{
		Matrix4 Result;

		const float cos = Cosine(A.Q);
		const float sin = Sine(A.Q);
		const float omc = 1.0f - cos;

		Result[0] = A.X * omc + cos;
		Result[1] = A.Y * A.X * omc - A.Y * sin;
		Result[2] = A.X * A.Z * omc - A.Y * sin;

		Result[4] = A.X * A.Y * omc - A.Z * sin;
		Result[5] = A.Y * omc + cos;
		Result[6] = A.Y * A.Z * omc + A.X * sin;

		Result[8] = A.X * A.Z * omc + A.Y * sin;
		Result[9] = A.Y * A.Z * omc - A.X * sin;
		Result[10] = A.Z * omc + cos;

		return Result;
	}

	INLINE static void Scale(Matrix4 & A, const Vector3 & B)
	{
		A[0] = B.X;
		A[5] = B.Y;
		A[10] = B.Z;

		return;
	}

	INLINE static Matrix4 Scaling(const Vector3 & A)
	{
		Matrix4 Result;

		Result[0] = A.X;
		Result[5] = A.Y;
		Result[10] = A.Z;

		return Result;
	}

	INLINE static Matrix4 Transformation(const Transform3& _A)
	{
		Matrix4 Return;
		Translate(Return, _A.Position);
		Return *= Rotation(_A.Rotation);
		Return *= Scaling(_A.Scale);
		return Return;
	}

	INLINE static float Clamp(float _A, float _Min, float _Max)
	{
		return _A > _Max ? _Max : _A < _Min ? _Min : _A;
	}

	INLINE static Vector3 ClosestPointOnPlane(const Vector3& _Point, const Plane& _Plane)
	{
		const float T = (Dot(_Plane.Normal, _Point) - _Plane.D) / Dot(_Plane.Normal, _Plane.Normal);
		return _Point - _Plane.Normal * T;
	}

	INLINE static double DistanceFromPointToPlane(const Vector3& _Point, const Plane& _Plane)
	{
		// return Dot(q, p.n) - p.d; if plane equation normalized (||p.n||==1)
		return (Dot(_Plane.Normal, _Point) - _Plane.D) / Dot(_Plane.Normal, _Plane.Normal);
	}

	INLINE static void ClosestPointOnLineSegmentToPoint(const Vector3& _C, const Vector3& _A, const Vector3& _B, double& _T, Vector3& _D)
	{
		Vector3 AB = _B - _A;
		// Project c onto ab, computing parameterized position d(t) = a + t*(b – a)
		_T = Dot(_C - _A, AB) / Dot(AB, AB);
		// If outside segment, clamp t (and therefore d) to the closest endpoint
		if (_T < 0.0) _T = 0.0;
		if (_T > 1.0) _T = 1.0;
		// Compute projected position from the clamped t
		_D = _A + AB * _T;
	}

	INLINE static double SquaredDistancePointToSegment(const Vector3& _A, const Vector3& _B, const Vector3& _C)
	{
		Vector3 AB = _B - _A, AC = _C - _A, BC = _C - _B;
		float E = Dot(AC, AB);
		// Handle cases where c projects outside ab
		if (E <= 0.0f) return Dot(AC, AC);
		float f = Dot(AB, AB);
		if (E >= f) return Dot(BC, BC);
		// Handle cases where c projects onto ab
		return Dot(AC, AC) - E * E / f;
	}

	INLINE static Vector3 ClosestPointOnTriangleToPoint(const Vector3& _A, const Vector3& _P1, const Vector3& _P2, const Vector3& _P3)
	{
		// Check if P in vertex region outside A
		const Vector3 AP = _A - _P1;
		const Vector3 AB = _P2 - _P1;
		const Vector3 AC = _P3 - _P1;

		const float D1 = Dot(AB, AP);
		const float D2 = Dot(AC, AP);
		if (D1 <= 0.0f && D2 <= 0.0f) return _P1; // barycentric coordinates (1,0,0)

		// Check if P in vertex region outside B
		const Vector3 BP = _A - _P2;
		const float D3 = Dot(AB, BP);
		const float D4 = Dot(AC, BP);
		if (D3 >= 0.0f && D4 <= D3) return _P2; // barycentric coordinates (0,1,0)

		// Check if P in edge region of AB, if so return projection of P onto AB
		const float VC = D1 * D4 - D3 * D2;
		if (VC <= 0.0f && D1 >= 0.0f && D3 <= 0.0f)
		{
			const float V = D1 / (D1 - D3);
			return _P1 + AB * V; // barycentric coordinates (1-v,v,0)
		}

		// Check if P in vertex region outside C
		const Vector3 CP = _A - _P3;
		const float D5 = Dot(AB, CP);
		const float D6 = Dot(AC, CP);
		if (D6 >= 0.0f && D5 <= D6) return _P3; // barycentric coordinates (0,0,1)

		// Check if P in edge region of AC, if so return projection of P onto AC
		const float VB = D5 * D2 - D1 * D6;
		if (VB <= 0.0f && D2 >= 0.0f && D6 <= 0.0f)
		{
			const float W = D2 / (D2 - D6);
			return _P1 + AC * W; // barycentric coordinates (1-w,0,w)
		}

		// Check if P in edge region of BC, if so return projection of P onto BC
		float VA = D3 * D6 - D5 * D4;
		if (VA <= 0.0f && (D4 - D3) >= 0.0f && (D5 - D6) >= 0.0f)
		{
			const float W = (D4 - D3) / ((D4 - D3) + (D5 - D6));
			return _P2 + (_P3 - _P2) * W; // barycentric coordinates (0,1-w,w)
		}

		// P inside face region. Compute Q through its barycentric coordinates (u,v,w)
		const float Denom = 1.0f / (VA + VB + VC);
		const float V = VB * Denom;
		const float W = VC * Denom;
		return _P1 + AB * V + AC * W; // = u*a + v*b + w*c, u = va * denom = 1.0f - v - w
	}

	INLINE static bool PointOutsideOfPlane(const Vector3& p, const Vector3& a, const Vector3& b, const Vector3& c)
	{
		return Dot(p - a, Cross(b - a, c - a)) >= 0.0f; // [AP AB AC] >= 0
	}

	INLINE static bool PointOutsideOfPlane(const Vector3& p, const Vector3& a, const Vector3& b, const Vector3& c, const Vector3& d)
	{
		const float signp = Dot(p - a, Cross(b - a, c - a)); // [AP AB AC]
		const float signd = Dot(d - a, Cross(b - a, c - a)); // [AD AB AC]
		// Points on opposite sides if expression signs are opposite
		return signp * signd < 0.0f;
	}

	INLINE static Vector3 ClosestPtPointTetrahedron(const Vector3& p, const Vector3& a, const Vector3& b, const Vector3& c, const Vector3& d)
	{
		// Start out assuming point inside all halfspaces, so closest to itself
		Vector3 ClosestPoint = p;
		float BestSquaredDistance = 3.402823466e+38F;

		// If point outside face abc then compute closest point on abc
		if (PointOutsideOfPlane(p, a, b, c))
		{
			const Vector3 q = ClosestPointOnTriangleToPoint(p, a, b, c);
			const float sqDist = Dot(q - p, q - p);
			// Update best closest point if (squared) distance is less than current best
			if (sqDist < BestSquaredDistance) BestSquaredDistance = sqDist, ClosestPoint = q;
		}

		// Repeat test for face acd
		if (PointOutsideOfPlane(p, a, c, d))
		{
			const Vector3 q = ClosestPointOnTriangleToPoint(p, a, c, d);
			const float sqDist = Dot(q - p, q - p);
			if (sqDist < BestSquaredDistance) BestSquaredDistance = sqDist, ClosestPoint = q;
		}

		// Repeat test for face adb
		if (PointOutsideOfPlane(p, a, d, b))
		{
			const Vector3 q = ClosestPointOnTriangleToPoint(p, a, d, b);
			const float sqDist = Dot(q - p, q - p);
			if (sqDist < BestSquaredDistance) BestSquaredDistance = sqDist, ClosestPoint = q;
		}

		// Repeat test for face bdc
		if (PointOutsideOfPlane(p, b, d, c))
		{
			const Vector3 q = ClosestPointOnTriangleToPoint(p, b, d, c);
			const float sqDist = Dot(q - p, q - p);
			if (sqDist < BestSquaredDistance) BestSquaredDistance = sqDist, ClosestPoint = q;
		}

		return ClosestPoint;
	}
};
